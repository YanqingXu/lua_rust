/**
 * Read-only execution-trace adapter for the pinned lua_cpp oracle.
 *
 * The pinned CLI parses --trace but installs its sink on a legacy singleton,
 * while script execution uses an EngineContext-owned GlobalState.  This
 * checkout-external adapter links the pristine installed library and installs
 * a sink on the actual EngineContext.  It supports snapshot and instruction-
 * diff evidence plus the pinned VM's real CALL/RETURN lifecycle events.  It
 * reconstructs Upvalue open/close evidence from real CLOSURE pseudo-
 * instructions and frame-close opcodes without changing VM/compiler behavior.
 * Top-level unhandled errors are recorded at the protected execution boundary
 * because the pinned VM defines Error events but does not currently publish
 * them from an opcode handler.
 */

#include "compiler/opcode.hpp"
#include "core/function.hpp"
#include "core/thread.hpp"
#include "core/upvalue.hpp"
#include "debug/trace_sink.hpp"
#include "debug/trace_types.hpp"
#include "debug/value_serializer.hpp"
#include "lib/lib_manager.hpp"
#include "runtime/runtime_services.hpp"
#include "vm/state/call_info.hpp"
#include "vm/state/global_state.hpp"
#include "vm/state/lua_state.hpp"
#include "vm/state/stack.hpp"
#include "vm/vm.hpp"

#include "lauxlib.h"
#include "lua.h"

#include <algorithm>
#include <cstdlib>
#include <format>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

using namespace Lua;

namespace {

class StateRetirementTracker {
public:
    void observe(LuaState* state) {
        if (state == nullptr) {
            return;
        }
        const auto active = std::find_if(observed_.rbegin(), observed_.rend(), [&](const Observation& entry) {
            return entry.state == state && !entry.retired;
        });
        if (active == observed_.rend()) {
            observed_.push_back(Observation{state});
        }
    }

    template <typename Consumer> void drain(Consumer&& consumer) {
        for (Observation& entry : observed_) {
            if (entry.retired && !entry.delivered) {
                entry.delivered = true;
                consumer(entry.state);
            }
        }
    }

    static void* allocate(void* userData, void* pointer, std::size_t, std::size_t newSize) noexcept {
        auto* tracker = static_cast<StateRetirementTracker*>(userData);
        if (newSize == 0) {
            if (tracker != nullptr && pointer != nullptr) {
                const auto active = std::find_if(tracker->observed_.rbegin(), tracker->observed_.rend(),
                                                 [&](const Observation& entry) {
                                                     return entry.state == pointer && !entry.retired;
                                                 });
                if (active != tracker->observed_.rend()) {
                    active->retired = true;
                }
            }
            std::free(pointer);
            return nullptr;
        }
        return pointer == nullptr ? std::malloc(newSize) : std::realloc(pointer, newSize);
    }

private:
    struct Observation {
        LuaState* state = nullptr;
        bool retired = false;
        bool delivered = false;
    };

    std::vector<Observation> observed_;
};

Str serializeChangedRegisters(const Vec<TraceRegisterChange>& changes) {
    Str output = "[";
    for (usize index = 0; index < changes.size(); ++index) {
        if (index > 0) {
            output += ",";
        }
        const TraceRegisterChange& change = changes[index];
        output += "{\"slot\":" + std::to_string(change.slot) + ",\"name\":";
        output += change.hasName ? "\"" + Trace::jsonEscape(change.name) + "\"" : "null";
        output += ",\"old\":" + (change.oldValue.empty() ? Str("null") : change.oldValue);
        output += ",\"new\":" + (change.newValue.empty() ? Str("null") : change.newValue);
        output += ",\"oldType\":\"" + Trace::jsonEscape(change.oldType != nullptr ? change.oldType : "unknown") +
                  "\",\"newType\":\"" +
                  Trace::jsonEscape(change.newType != nullptr ? change.newType : "unknown") + "\"}";
    }
    output += "]";
    return output;
}

class VmEvidenceJsonSink final : public ITraceSink {
public:
    VmEvidenceJsonSink(const std::string& path, LuaState* state, bool diffMode, StateRetirementTracker& retirements)
        : file_(path, std::ios::out | std::ios::trunc), state_(state), diffMode_(diffMode),
          retirements_(retirements) {}

    [[nodiscard]] bool isOpen() const noexcept {
        return file_.is_open();
    }

    [[nodiscard]] u64 eventCount() const noexcept {
        return sequence_;
    }

    void onInstruction(const TraceEvent& event) override {
        if (!file_.is_open()) {
            return;
        }

        drainStateRetirements();
        if (!diffMode_) {
            flushRetirementEvents();
        }
        LuaState* currentState = activeState();
        observeStateIdentity(currentState);
        const ExecutionPoint point = instructionPoint(event);
        const InstructionEvidence currentInstruction = instructionEvidence(currentState, event);
        if (diffMode_) {
            reconcileTrackedUpvalues(currentState, &currentInstruction, true);
        } else {
            reconcileTrackedUpvalues(currentState, lastInstruction(currentState));
        }
        reconcileExecutionState(currentState, point);
        const i32 stackTop = traceStackTop(currentState, event);
        const Str evidence = event.includeChangedRegisters
                                 ? ",\"changedRegisters\":" + serializeChangedRegisters(event.changedRegisters)
                                 : ",\"registers\":" +
                                       (event.base != nullptr && event.maxStack > 0
                                            ? Trace::serializeRegisters(event.base, event.maxStack, event.proto, event.pc)
                                            : Str("[]"));
        file_ << std::format(
            "{{\"seq\":{},\"kind\":\"instruction\",\"funcName\":\"{}\",\"pc\":{},\"op\":\"{}\","
            "\"a\":{},\"b\":{},\"c\":{},\"bx\":{},\"sbx\":{},\"line\":{},"
            "\"source\":\"{}\",\"callDepth\":{},\"stackTop\":{}{}}}\n",
            sequence_, Trace::jsonEscape(event.funcName), event.pc, getOpName(event.op), event.a, event.b, event.c,
            event.bx, event.sbx, event.line, Trace::jsonEscape(event.source != nullptr ? event.source : "?"),
            event.callDepth, stackTop, evidence);
        ++sequence_;

        rememberExecutionPoint(currentState, point);
        rememberInstruction(currentInstruction);
        discoverClosureUpvalues(currentState, event);
        if (diffMode_) {
            reconcileTrackedUpvalues(currentState, &currentInstruction, false);
            flushRetirementEvents();
        }
    }

    void onCall(const TraceEvent& event) override {
        drainStateRetirements();
        flushRetirementEvents();
        LuaState* currentState = activeState();
        observeStateIdentity(currentState);
        const ExecutionPoint point = activeFramePoint(currentState, event, std::max(0, event.callDepth - 1));
        reconcileTrackedUpvalues(currentState);
        reconcileExecutionState(currentState, point);
        writeControlEvent("call", event);
        rememberExecutionPoint(currentState, point);
    }

    void onReturn(const TraceEvent& event) override {
        drainStateRetirements();
        flushRetirementEvents();
        LuaState* currentState = activeState();
        observeStateIdentity(currentState);
        const ExecutionPoint point = activeFramePoint(currentState, event, event.callDepth);
        reconcileTrackedUpvalues(currentState);
        reconcileExecutionState(currentState, point);
        writeControlEvent("return", event);
        rememberExecutionPoint(currentState, point);
    }

    void onError(const TraceEvent& event) override {
        drainStateRetirements();
        flushRetirementEvents();
        LuaState* currentState = activeState();
        observeStateIdentity(currentState);
        const ExecutionPoint point = activeFramePoint(currentState, event, event.callDepth);
        reconcileTrackedUpvalues(currentState);
        reconcileExecutionState(currentState, point);
        writeErrorEvent(event.funcName, event.source, event.line, event.callDepth,
                        event.errorMsg != nullptr ? event.errorMsg : "runtime error");
        rememberExecutionPoint(currentState, point);
    }

    void emitTopLevelError(const std::string& message) {
        drainStateRetirements();
        flushRetirementEvents();
        LuaState* currentState = activeState();
        observeStateIdentity(currentState);
        const ExecutionPoint point = activeFramePoint(currentState);
        reconcileTrackedUpvalues(currentState);
        reconcileExecutionState(currentState, point);
        writeErrorEvent("?", "?", 0, 0, message);
    }

    void flush() override {
        drainStateRetirements();
        flushRetirementEvents();
        LuaState* currentState = activeState();
        observeStateIdentity(currentState);
        reconcileTrackedUpvalues(currentState);
        reconcileExecutionState(currentState, activeFramePoint(currentState));
        file_.flush();
    }

private:
    struct StateIdentity {
        LuaState* state = nullptr;
        Thread* thread = nullptr;
        Function* suspendedFunction = nullptr;
        const Proto* suspendedProto = nullptr;
        i32 suspendedPc = -1;
        bool suspended = false;
        bool terminal = false;
    };

    struct ContinuationFingerprint {
        Function* function = nullptr;
        const Proto* proto = nullptr;
        i32 pc = -1;
    };

    struct OpenUpvalueEvidence {
        LuaState* state = nullptr;
        Thread* thread = nullptr;
        usize absoluteSlot = 0;
        usize frameBase = 0;
        const Proto* ownerProto = nullptr;
        Str ownerFunction;
        Str ownerSource;
        i32 ownerLine = 0;
        i32 callDepth = 0;
        i32 slot = 0;
        Str name;
        bool published = false;
    };

    struct LifecycleEvidence {
        const char* kind = "upvalue-open";
        Str funcName;
        Str source;
        i32 line = 0;
        i32 callDepth = 0;
        i32 slot = 0;
        Str name;
    };

    struct ExecutionStateEvidence {
        LuaState* state = nullptr;
        Thread* thread = nullptr;
        Str functionName;
        Str source;
        i32 line = 0;
        i32 callDepth = 0;
    };

    struct ExecutionPoint {
        Str functionName = "?";
        Str source = "?";
        i32 line = 0;
        i32 callDepth = 0;
    };

    struct LuaFrameEvidence {
        bool valid = false;
        usize frame = 0;
        usize base = 0;
        usize top = 0;
        Proto* proto = nullptr;
    };

    struct StackTopProjection {
        LuaState* state = nullptr;
        Thread* thread = nullptr;
        usize frame = 0;
        const Proto* proto = nullptr;
        i32 callDepth = 0;
        i32 top = 0;
    };

    struct InstructionEvidence {
        LuaState* state = nullptr;
        Thread* thread = nullptr;
        usize frame = 0;
        usize frameBase = 0;
        const Proto* proto = nullptr;
        Str functionName;
        Str source;
        i32 line = 0;
        i32 callDepth = 0;
        i32 pc = 0;
        i32 maxStack = 0;
        OpCode op = OpCode::MOVE;
        i32 a = 0;
        i32 b = 0;
        i32 c = 0;
    };

    static Str traceSource(const TraceEvent& event) {
        return event.source != nullptr ? event.source : "?";
    }

    static Str protoSource(const Proto* proto) {
        return proto != nullptr && proto->getSource() != nullptr ? proto->getSource()->c_str() : "?";
    }

    static Str protoFunctionName(const Proto* proto) {
        if (proto == nullptr) {
            return "?";
        }
        const Str source = protoSource(proto);
        return proto->getLineDefined() > 0 ? std::format("{}:{}", source, proto->getLineDefined()) : source;
    }

    static Str upvalueName(Proto* proto, usize index) {
        if (proto == nullptr) {
            return "?";
        }
        GCString* name = proto->getUpvalueName(index);
        return name != nullptr ? name->c_str() : "?";
    }

    static Proto* frameProto(LuaState* currentState, usize frameIndex) {
        if (currentState == nullptr || frameIndex >= currentState->getCallStack().size()) {
            return nullptr;
        }
        const CallInfo& frame = currentState->getCallStack()[frameIndex];
        Stack& stack = currentState->getStack();
        if (frame.func >= stack.size()) {
            return nullptr;
        }
        const Value& functionValue = stack[frame.func];
        if (!functionValue.isFunction()) {
            return nullptr;
        }
        Function* function = functionValue.asFunction();
        return function != nullptr ? function->getProto() : nullptr;
    }

    static ContinuationFingerprint rootLuaContinuation(LuaState* currentState) {
        if (currentState == nullptr) {
            return {};
        }
        Stack& stack = currentState->getStack();
        for (usize frameIndex = 0; frameIndex <= currentState->getCurrentCI() &&
                                   frameIndex < currentState->getCallStack().size();
             ++frameIndex) {
            const CallInfo& frame = currentState->getCallStack()[frameIndex];
            if (frame.func >= stack.size() || !stack[frame.func].isFunction()) {
                continue;
            }
            Function* function = stack[frame.func].asFunction();
            Proto* proto = function != nullptr ? function->getProto() : nullptr;
            if (proto == nullptr) {
                continue;
            }
            i32 pc = -1;
            const auto code = proto->getInstructionSpan();
            if (frame.savedpc != nullptr && !code.empty() && frame.savedpc >= code.data() &&
                frame.savedpc <= code.data() + code.size()) {
                pc = static_cast<i32>(frame.savedpc - code.data());
            }
            return ContinuationFingerprint{function, proto, pc};
        }
        return {};
    }

    static LuaFrameEvidence activeLuaFrame(LuaState* currentState) {
        if (currentState == nullptr) {
            return {};
        }
        usize frameIndex = currentState->getCurrentCI() + 1;
        while (frameIndex > 0) {
            --frameIndex;
            Proto* proto = frameProto(currentState, frameIndex);
            if (proto == nullptr) {
                continue;
            }
            const CallInfo& frame = currentState->getCallStack()[frameIndex];
            return LuaFrameEvidence{true, frameIndex, frame.base, frame.top, proto};
        }
        return {};
    }

    static bool findActiveEventFrame(LuaState* currentState, const Proto* proto, usize& frameIndex) {
        if (currentState == nullptr || proto == nullptr) {
            return false;
        }
        usize candidate = currentState->getCurrentCI() + 1;
        while (candidate > 0) {
            --candidate;
            if (frameProto(currentState, candidate) == proto) {
                frameIndex = candidate;
                return true;
            }
        }
        return false;
    }

    static i32 frameCapacity(const LuaFrameEvidence& frame) {
        return frame.valid && frame.top >= frame.base ? static_cast<i32>(frame.top - frame.base) : 0;
    }

    static i32 rawFrameTop(LuaState* currentState, const LuaFrameEvidence& frame) {
        if (currentState == nullptr || !frame.valid || currentState->getAbsoluteTop() <= frame.base) {
            return 0;
        }
        return static_cast<i32>(currentState->getAbsoluteTop() - frame.base);
    }

    static Thread* stateThread(LuaState* currentState) noexcept {
        return currentState != nullptr ? currentState->getThread() : nullptr;
    }

    static bool sameIdentity(LuaState* leftState, Thread* leftThread, LuaState* rightState,
                             Thread* rightThread) noexcept {
        return leftState == rightState && leftThread == rightThread;
    }

    static bool sameIdentity(LuaState* leftState, Thread* leftThread, LuaState* currentState) noexcept {
        return sameIdentity(leftState, leftThread, currentState, stateThread(currentState));
    }

    static InstructionEvidence instructionEvidence(LuaState* currentState, const TraceEvent& event) {
        const LuaFrameEvidence frame = activeLuaFrame(currentState);
        return InstructionEvidence{currentState,
                                   stateThread(currentState),
                                   frame.valid ? frame.frame : 0,
                                   frame.valid ? frame.base : 0,
                                   event.proto,
                                   event.funcName,
                                   traceSource(event),
                                   event.line,
                                   event.callDepth,
                                   event.pc,
                                   event.maxStack,
                                   event.op,
                                   event.a,
                                   event.b,
                                   event.c};
    }

    const InstructionEvidence* lastInstruction(LuaState* currentState) const {
        Thread* currentThread = stateThread(currentState);
        const auto found = std::find_if(lastInstructions_.begin(), lastInstructions_.end(),
                                        [&](const auto& entry) {
                                            return sameIdentity(entry.state, entry.thread, currentState,
                                                                currentThread);
                                        });
        return found != lastInstructions_.end() ? &*found : nullptr;
    }

    void rememberInstruction(const InstructionEvidence& evidence) {
        auto found = std::find_if(lastInstructions_.begin(), lastInstructions_.end(),
                                  [&](const auto& entry) {
                                      return sameIdentity(entry.state, entry.thread, evidence.state,
                                                          evidence.thread);
                                  });
        if (found != lastInstructions_.end()) {
            *found = evidence;
            return;
        }
        lastInstructions_.push_back(evidence);
    }

    StackTopProjection& projectionForEvent(LuaState* currentState, const TraceEvent& event, usize frameHint) {
        Thread* currentThread = stateThread(currentState);
        auto exact = std::find_if(stackTopProjections_.begin(), stackTopProjections_.end(), [&](const auto& entry) {
            return sameIdentity(entry.state, entry.thread, currentState, currentThread) &&
                   entry.frame == frameHint && entry.proto == event.proto;
        });
        if (exact != stackTopProjections_.end()) {
            exact->callDepth = event.callDepth;
            return *exact;
        }
        auto historical =
            std::find_if(stackTopProjections_.begin(), stackTopProjections_.end(), [&](const auto& entry) {
                return sameIdentity(entry.state, entry.thread, currentState, currentThread) &&
                       entry.proto == event.proto && entry.callDepth == event.callDepth;
            });
        if (historical != stackTopProjections_.end()) {
            return *historical;
        }
        stackTopProjections_.push_back(
            StackTopProjection{currentState, currentThread, frameHint, event.proto, event.callDepth, event.maxStack});
        return stackTopProjections_.back();
    }

    void setProjectionTop(LuaState* currentState, const LuaFrameEvidence& frame, i32 callDepth, i32 top) {
        if (!frame.valid) {
            return;
        }
        Thread* currentThread = stateThread(currentState);
        auto existing = std::find_if(stackTopProjections_.begin(), stackTopProjections_.end(), [&](const auto& entry) {
            return sameIdentity(entry.state, entry.thread, currentState, currentThread) &&
                   entry.frame == frame.frame && entry.proto == frame.proto;
        });
        if (existing != stackTopProjections_.end()) {
            existing->callDepth = callDepth;
            existing->top = top;
            return;
        }
        stackTopProjections_.push_back(
            StackTopProjection{currentState, currentThread, frame.frame, frame.proto, callDepth, top});
    }

    void discardProjectionsAbove(LuaState* currentState, usize frame) {
        Thread* currentThread = stateThread(currentState);
        std::erase_if(stackTopProjections_, [&](const auto& entry) {
            return sameIdentity(entry.state, entry.thread, currentState, currentThread) && entry.frame > frame;
        });
    }

    i32 traceSnapshotStackTop(LuaState* currentState, const TraceEvent& event) {
        const LuaFrameEvidence activeFrame = activeLuaFrame(currentState);
        if (!activeFrame.valid) {
            return event.maxStack;
        }

        const InstructionEvidence* previous = lastInstruction(currentState);
        if (previous == nullptr) {
            const i32 capacity = frameCapacity(activeFrame);
            setProjectionTop(currentState, activeFrame, event.callDepth, capacity);
            return capacity;
        }

        if (activeFrame.frame > previous->frame) {
            const i32 capacity = frameCapacity(activeFrame);
            setProjectionTop(currentState, activeFrame, event.callDepth, capacity);
            return capacity;
        }
        if (activeFrame.frame < previous->frame) {
            const i32 callerTop = rawFrameTop(currentState, activeFrame);
            discardProjectionsAbove(currentState, activeFrame.frame);
            setProjectionTop(currentState, activeFrame, event.callDepth, callerTop);
            return callerTop;
        }
        if (activeFrame.proto != previous->proto) {
            Thread* currentThread = stateThread(currentState);
            std::erase_if(stackTopProjections_, [&](const auto& entry) {
                return sameIdentity(entry.state, entry.thread, currentState, currentThread) &&
                       entry.frame == activeFrame.frame && entry.proto != activeFrame.proto;
            });
            const i32 capacity = frameCapacity(activeFrame);
            setProjectionTop(currentState, activeFrame, event.callDepth, capacity);
            return capacity;
        }

        StackTopProjection& projection = projectionForEvent(currentState, event, activeFrame.frame);
        switch (previous->op) {
        case OpCode::CALL:
            projection.top = previous->c == 0 ? rawFrameTop(currentState, activeFrame) : frameCapacity(activeFrame);
            break;
        case OpCode::VARARG:
            if (previous->b == 0) {
                projection.top = rawFrameTop(currentState, activeFrame);
            }
            break;
        case OpCode::TFORLOOP:
            projection.top = frameCapacity(activeFrame);
            break;
        case OpCode::SETLIST:
            if (previous->b == 0) {
                projection.top = frameCapacity(activeFrame);
            }
            break;
        case OpCode::TAILCALL:
            projection.top = event.pc == 0 ? frameCapacity(activeFrame) : rawFrameTop(currentState, activeFrame);
            break;
        default:
            break;
        }
        return projection.top;
    }

    i32 traceStackTop(LuaState* currentState, const TraceEvent& event) {
        if (currentState == nullptr) {
            return 0;
        }
        if (currentState == state_) {
            return currentState->getTop();
        }
        if (!diffMode_) {
            return traceSnapshotStackTop(currentState, event);
        }

        const LuaFrameEvidence activeFrame = activeLuaFrame(currentState);
        usize ownerFrame = activeFrame.valid ? activeFrame.frame : currentState->getCurrentCI();
        const bool ownerStillActive = findActiveEventFrame(currentState, event.proto, ownerFrame);
        StackTopProjection& ownerProjection = projectionForEvent(currentState, event, ownerFrame);
        const i32 topBefore = ownerProjection.top;

        if (activeFrame.valid && activeFrame.frame > ownerProjection.frame) {
            const i32 childCapacity = frameCapacity(activeFrame);
            setProjectionTop(currentState, activeFrame, event.callDepth + 1, childCapacity);
            return childCapacity;
        }
        if (activeFrame.valid && activeFrame.frame < ownerProjection.frame) {
            const i32 callerTop = rawFrameTop(currentState, activeFrame);
            discardProjectionsAbove(currentState, activeFrame.frame);
            setProjectionTop(currentState, activeFrame, std::max(0, event.callDepth - 1), callerTop);
            return callerTop;
        }

        i32 nextTop = ownerProjection.top;
        switch (event.op) {
        case OpCode::CALL:
            nextTop = event.c != 0 ? event.maxStack : currentState->getTop();
            break;
        case OpCode::VARARG:
            nextTop = event.b == 0 ? currentState->getTop() : ownerProjection.top;
            break;
        case OpCode::TFORLOOP:
            nextTop = event.maxStack;
            break;
        case OpCode::SETLIST:
            nextTop = event.b == 0 ? event.maxStack : ownerProjection.top;
            break;
        case OpCode::TAILCALL:
            nextTop = activeFrame.valid && (!ownerStillActive || activeFrame.proto != event.proto)
                          ? frameCapacity(activeFrame)
                          : rawFrameTop(currentState, activeFrame);
            break;
        case OpCode::RETURN:
            nextTop = event.b > 0 ? std::max(0, event.b - 1) : std::max(0, topBefore - event.a);
            break;
        default:
            break;
        }

        if (event.op == OpCode::TAILCALL && activeFrame.valid && activeFrame.proto != event.proto) {
            Thread* currentThread = stateThread(currentState);
            std::erase_if(stackTopProjections_, [&](const auto& entry) {
                return sameIdentity(entry.state, entry.thread, currentState, currentThread) &&
                       entry.frame == activeFrame.frame && entry.proto != activeFrame.proto;
            });
            setProjectionTop(currentState, activeFrame, event.callDepth, nextTop);
        } else {
            ownerProjection.top = nextTop;
        }
        return nextTop;
    }

    void retireStateIdentity(const StateIdentity& identity) {
        const auto belongsToIdentity = [&](const auto& entry) {
            return sameIdentity(entry.state, entry.thread, identity.state, identity.thread);
        };
        std::erase_if(openUpvalues_, belongsToIdentity);
        std::erase_if(executionStates_, belongsToIdentity);
        std::erase_if(stackTopProjections_, belongsToIdentity);
        std::erase_if(lastInstructions_, belongsToIdentity);
        std::erase_if(stateIdentities_, [&](const StateIdentity& entry) {
            return sameIdentity(entry.state, entry.thread, identity.state, identity.thread);
        });
    }

    void retireDeallocatedState(LuaState* state) {
        std::vector<OpenUpvalueEvidence> retiredOpenUpvalues;
        auto open = openUpvalues_.begin();
        while (open != openUpvalues_.end()) {
            if (open->state != state) {
                ++open;
                continue;
            }
            retiredOpenUpvalues.push_back(*open);
            open = openUpvalues_.erase(open);
        }

        for (const OpenUpvalueEvidence& entry : retiredOpenUpvalues) {
            if (!entry.published) {
                pendingRetirementEvents_.push_back(LifecycleEvidence{
                    "upvalue-open", entry.ownerFunction, entry.ownerSource, entry.ownerLine, entry.callDepth,
                    entry.slot, entry.name});
            }
        }
        std::sort(retiredOpenUpvalues.begin(), retiredOpenUpvalues.end(), [](const auto& left, const auto& right) {
            return left.absoluteSlot > right.absoluteSlot;
        });
        for (const OpenUpvalueEvidence& entry : retiredOpenUpvalues) {
            pendingRetirementEvents_.push_back(LifecycleEvidence{
                "upvalue-close", entry.ownerFunction, entry.ownerSource, entry.ownerLine, entry.callDepth,
                entry.slot, entry.name});
        }

        const auto belongsToState = [&](const auto& entry) { return entry.state == state; };
        std::erase_if(executionStates_, belongsToState);
        std::erase_if(stackTopProjections_, belongsToState);
        std::erase_if(lastInstructions_, belongsToState);
        std::erase_if(stateIdentities_, [&](const StateIdentity& entry) { return entry.state == state; });
    }

    void drainStateRetirements() {
        retirements_.drain([&](LuaState* state) { retireDeallocatedState(state); });
    }

    void flushRetirementEvents() {
        for (const LifecycleEvidence& event : pendingRetirementEvents_) {
            writeLifecycleEvent(event);
        }
        pendingRetirementEvents_.clear();
    }

    static bool isReincarnatedState(const StateIdentity& identity, LuaState* currentState) {
        if (identity.terminal) {
            return true;
        }
        if (!identity.suspended) {
            return false;
        }
        const ContinuationFingerprint current = rootLuaContinuation(currentState);
        if (identity.suspendedFunction != nullptr && current.function != nullptr &&
            identity.suspendedFunction != current.function) {
            return true;
        }
        if (identity.suspendedProto != nullptr && current.proto != nullptr && identity.suspendedProto != current.proto) {
            return true;
        }
        return identity.suspendedProto != nullptr && identity.suspendedProto == current.proto &&
               identity.suspendedPc >= 0 && current.pc >= 0 && current.pc < identity.suspendedPc;
    }

    void rememberInactiveState(const ExecutionStateEvidence& execution, bool terminal) {
        auto identity = std::find_if(stateIdentities_.begin(), stateIdentities_.end(), [&](const auto& entry) {
            return sameIdentity(entry.state, entry.thread, execution.state, execution.thread);
        });
        if (identity == stateIdentities_.end()) {
            return;
        }
        const ContinuationFingerprint continuation = rootLuaContinuation(execution.state);
        identity->suspendedFunction = continuation.function;
        identity->suspendedProto = continuation.proto;
        identity->suspendedPc = continuation.pc;
        identity->suspended = !terminal;
        identity->terminal = terminal;
    }

    void observeStateIdentity(LuaState* currentState) {
        if (currentState == nullptr) {
            return;
        }
        retirements_.observe(currentState);
        Thread* currentThread = stateThread(currentState);
        auto exact = std::find_if(stateIdentities_.begin(), stateIdentities_.end(), [&](const auto& entry) {
            return sameIdentity(entry.state, entry.thread, currentState, currentThread);
        });
        if (exact != stateIdentities_.end() && !isReincarnatedState(*exact, currentState)) {
            exact->suspended = false;
            exact->terminal = false;
            return;
        }
        if (exact != stateIdentities_.end()) {
            const StateIdentity obsolete = *exact;
            retireStateIdentity(obsolete);
        }

        /**
         * LuaState has no public destruction callback or generation number.  A
         * newly observed state/thread pairing nevertheless proves that any
         * cached pairing which reused either non-null component is obsolete:
         * one live LuaState cannot change its owning Thread, and one live
         * Thread cannot change its LuaState.  A suspended continuation
         * fingerprint covers the allocator's common case where both addresses
         * are reused together.  Retire obsolete evidence without synthesizing
         * a late close event and, importantly, without dereferencing the stale
         * pointer.
         */
        std::vector<StateIdentity> conflicts;
        for (const StateIdentity& identity : stateIdentities_) {
            const bool reusedState = identity.state == currentState && identity.thread != currentThread;
            const bool reusedThread = currentThread != nullptr && identity.thread == currentThread &&
                                      identity.state != currentState;
            if (reusedState || reusedThread) {
                conflicts.push_back(identity);
            }
        }
        for (const StateIdentity& identity : conflicts) {
            retireStateIdentity(identity);
        }
        stateIdentities_.push_back(StateIdentity{currentState, currentThread});
    }

    LuaState* activeState() const {
        if (state_ == nullptr) {
            return nullptr;
        }
        Thread* running = state_->getGlobalState().getRunningThread();
        return running != nullptr ? running->getLuaState() : state_;
    }

    static ExecutionPoint instructionPoint(const TraceEvent& event) {
        return ExecutionPoint{event.funcName, traceSource(event), event.line, event.callDepth};
    }

    static ExecutionPoint activeFramePoint(LuaState* currentState, const TraceEvent* fallback, i32 callDepth) {
        ExecutionPoint point;
        if (fallback != nullptr) {
            point = instructionPoint(*fallback);
        }
        if (callDepth >= 0) {
            point.callDepth = callDepth;
        }

        const LuaFrameEvidence frame = activeLuaFrame(currentState);
        if (!frame.valid || frame.proto == nullptr) {
            return point;
        }
        point.functionName = protoFunctionName(frame.proto);
        point.source = protoSource(frame.proto);
        const CallInfo& call = currentState->getCallStack()[frame.frame];
        const auto code = frame.proto->getInstructionSpan();
        if (call.savedpc != nullptr && !code.empty() && call.savedpc > code.data() &&
            call.savedpc <= code.data() + code.size()) {
            point.line = frame.proto->getLine(static_cast<usize>(call.savedpc - code.data() - 1));
        }
        return point;
    }

    static ExecutionPoint activeFramePoint(LuaState* currentState, const TraceEvent& fallback, i32 callDepth) {
        return activeFramePoint(currentState, &fallback, callDepth);
    }

    static ExecutionPoint activeFramePoint(LuaState* currentState) {
        return activeFramePoint(currentState, nullptr, 0);
    }

    void reconcileExecutionState(LuaState* currentState, const ExecutionPoint& point) {
        if (currentState == nullptr) {
            return;
        }
        Thread* currentThread = stateThread(currentState);
        if (executionStates_.empty()) {
            executionStates_.push_back(ExecutionStateEvidence{currentState, currentThread, point.functionName,
                                                               point.source, point.line, point.callDepth});
            return;
        }
        if (sameIdentity(executionStates_.back().state, executionStates_.back().thread, currentState,
                         currentThread)) {
            return;
        }

        reconcileTrackedUpvalues(executionStates_.back().state, nullptr, false, executionStates_.back().thread);

        Thread* suspendedThread = executionStates_.back().thread;
        const bool returnedToCaller = suspendedThread != nullptr &&
                                      (suspendedThread->getCoroutineStatus() == CoroutineStatus::Suspended ||
                                       suspendedThread->getCoroutineStatus() == CoroutineStatus::Dead);
        const auto ancestor = std::find_if(executionStates_.rbegin(), executionStates_.rend(),
                                           [&](const auto& entry) {
                                               return sameIdentity(entry.state, entry.thread, currentState,
                                                                   currentThread);
                                           });
        if (ancestor != executionStates_.rend() && returnedToCaller) {
            while (!executionStates_.empty() &&
                   !sameIdentity(executionStates_.back().state, executionStates_.back().thread, currentState,
                                 currentThread)) {
                ExecutionStateEvidence suspended = std::move(executionStates_.back());
                executionStates_.pop_back();
                Thread* thread = suspended.thread;
                if (thread != nullptr && thread->getCoroutineStatus() == CoroutineStatus::Suspended) {
                    rememberInactiveState(suspended, false);
                    writeCoroutineEvent("yield", suspended.functionName, suspended.source, suspended.line,
                                        suspended.callDepth);
                } else if (thread != nullptr && thread->getCoroutineStatus() == CoroutineStatus::Dead) {
                    rememberInactiveState(suspended, true);
                }
            }
            return;
        }

        executionStates_.push_back(ExecutionStateEvidence{currentState, currentThread, point.functionName,
                                                           point.source, point.line, point.callDepth});
        writeCoroutineEvent("resume", point.functionName, point.source, point.line, point.callDepth);
    }

    void rememberExecutionPoint(LuaState* currentState, const ExecutionPoint& point) {
        if (currentState == nullptr || executionStates_.empty() ||
            !sameIdentity(executionStates_.back().state, executionStates_.back().thread, currentState)) {
            return;
        }
        ExecutionStateEvidence& current = executionStates_.back();
        current.functionName = point.functionName;
        current.source = point.source;
        current.line = point.line;
        current.callDepth = point.callDepth;
    }

    void writeCoroutineEvent(const char* kind, const Str& funcName, const Str& source, i32 line, i32 callDepth) {
        if (!file_.is_open()) {
            return;
        }
        file_ << std::format(
            "{{\"seq\":{},\"kind\":\"{}\",\"funcName\":\"{}\",\"source\":\"{}\","
            "\"line\":{},\"callDepth\":{}}}\n",
            sequence_, kind, Trace::jsonEscape(funcName), Trace::jsonEscape(source), line, callDepth);
        ++sequence_;
    }

    static bool isActuallyOpen(const OpenUpvalueEvidence& entry, LuaState* eventState, Thread* eventThread) {
        if (eventState == nullptr ||
            !sameIdentity(entry.state, entry.thread, eventState, eventThread)) {
            return false;
        }
        for (Upvalue* upvalue = eventState->getOpenUpvalues(); upvalue != nullptr; upvalue = upvalue->getNext()) {
            if (upvalue->isOpen() && upvalue->getStackIndex() == entry.absoluteSlot) {
                return true;
            }
        }
        return false;
    }

    static bool closingInstructionMatches(const OpenUpvalueEvidence& entry, LuaState* eventState,
                                          Thread* eventThread, const InstructionEvidence* event) {
        if (event == nullptr || !sameIdentity(entry.state, entry.thread, eventState, eventThread) ||
            event->proto != entry.ownerProto ||
            event->callDepth != entry.callDepth) {
            return false;
        }
        if (event->op == OpCode::RETURN || event->op == OpCode::TAILCALL) {
            return true;
        }
        return event->op == OpCode::CLOSE && event->a >= 0 &&
               entry.absoluteSlot >= entry.frameBase + static_cast<usize>(event->a);
    }

    void discoverClosureUpvalues(LuaState* currentState, const TraceEvent& event) {
        if (currentState == nullptr || event.proto == nullptr || event.op != OpCode::CLOSURE || event.bx < 0 ||
            static_cast<usize>(event.bx) >= event.proto->getSubProtoCount()) {
            return;
        }
        Proto* child = event.proto->getSubProto(static_cast<usize>(event.bx));
        if (child == nullptr) {
            return;
        }
        Thread* currentThread = stateThread(currentState);
        const usize pseudoStart = static_cast<usize>(event.pc + 1);
        for (usize index = 0; index < child->getNumUpvalues(); ++index) {
            const usize pseudoPc = pseudoStart + index;
            if (pseudoPc >= event.proto->getInstructionCount()) {
                return;
            }
            const Instruction pseudo = event.proto->getInstruction(pseudoPc);
            if (GET_OPCODE(pseudo) != OpCode::MOVE) {
                continue;
            }
            const i32 slot = GETARG_B(pseudo);
            if (slot < 0) {
                continue;
            }
            const usize frameBase = currentState->getCurrentCallInfo().base;
            const usize absoluteSlot = frameBase + static_cast<usize>(slot);
            const auto existing = std::find_if(openUpvalues_.begin(), openUpvalues_.end(), [&](const auto& open) {
                return sameIdentity(open.state, open.thread, currentState, currentThread) &&
                       open.absoluteSlot == absoluteSlot;
            });
            if (existing != openUpvalues_.end()) {
                continue;
            }

            const Str name = upvalueName(child, index);
            openUpvalues_.push_back(OpenUpvalueEvidence{currentState, currentThread, absoluteSlot, frameBase,
                                                        event.proto, event.funcName, traceSource(event), event.line,
                                                        event.callDepth, slot, name, false});
        }
    }

    void reconcileTrackedUpvalues(LuaState* eventState = nullptr, const InstructionEvidence* event = nullptr,
                                  bool beforeInstruction = false, Thread* eventThread = nullptr) {
        if (eventState != nullptr && eventThread == nullptr) {
            eventThread = stateThread(eventState);
        }
        if (event == nullptr) {
            event = lastInstruction(eventState);
        }
        for (OpenUpvalueEvidence& entry : openUpvalues_) {
            if (!sameIdentity(entry.state, entry.thread, eventState, eventThread) || entry.published ||
                !isActuallyOpen(entry, eventState, eventThread)) {
                continue;
            }
            entry.published = true;
            writeLifecycleEvent(LifecycleEvidence{"upvalue-open", entry.ownerFunction, entry.ownerSource,
                                                  entry.ownerLine, entry.callDepth, entry.slot, entry.name});
        }

        std::vector<OpenUpvalueEvidence> closed;
        auto open = openUpvalues_.begin();
        while (open != openUpvalues_.end()) {
            if (!sameIdentity(open->state, open->thread, eventState, eventThread) || !open->published ||
                isActuallyOpen(*open, eventState, eventThread) ||
                (beforeInstruction && closingInstructionMatches(*open, eventState, eventThread, event))) {
                ++open;
                continue;
            }
            if (open->published) {
                closed.push_back(*open);
            }
            open = openUpvalues_.erase(open);
        }
        std::sort(closed.begin(), closed.end(), [](const auto& left, const auto& right) {
            return left.absoluteSlot > right.absoluteSlot;
        });
        for (const OpenUpvalueEvidence& entry : closed) {
            const bool useInstruction =
                !beforeInstruction && closingInstructionMatches(entry, eventState, eventThread, event);
            writeLifecycleEvent(LifecycleEvidence{"upvalue-close",
                                                  useInstruction ? event->functionName : entry.ownerFunction,
                                                  useInstruction ? event->source : entry.ownerSource,
                                                  useInstruction ? event->line : entry.ownerLine,
                                                  useInstruction ? event->callDepth : entry.callDepth, entry.slot,
                                                  entry.name});
        }
    }

    void writeLifecycleEvent(const LifecycleEvidence& event) {
        if (!file_.is_open()) {
            return;
        }
        file_ << std::format(
            "{{\"seq\":{},\"kind\":\"{}\",\"funcName\":\"{}\",\"source\":\"{}\","
            "\"line\":{},\"callDepth\":{},\"slot\":{},\"name\":\"{}\"}}\n",
            sequence_, event.kind, Trace::jsonEscape(event.funcName), Trace::jsonEscape(event.source), event.line,
            event.callDepth, event.slot, Trace::jsonEscape(event.name));
        ++sequence_;
    }

    void writeControlEvent(const char* kind, const TraceEvent& event) {
        if (!file_.is_open()) {
            return;
        }
        file_ << std::format(
            "{{\"seq\":{},\"kind\":\"{}\",\"funcName\":\"{}\",\"source\":\"{}\","
            "\"line\":{},\"callDepth\":{}}}\n",
            sequence_, kind, Trace::jsonEscape(event.funcName),
            Trace::jsonEscape(event.source != nullptr ? event.source : "?"), event.line, event.callDepth);
        ++sequence_;
    }

    void writeErrorEvent(const Str& funcName, const char* source, i32 line, i32 callDepth,
                         const std::string& message) {
        if (!file_.is_open()) {
            return;
        }
        const Str escapedMessage = Trace::jsonEscape(message);
        file_ << std::format(
            "{{\"seq\":{},\"kind\":\"error\",\"funcName\":\"{}\",\"message\":\"{}\","
            "\"errorValue\":\"{}\",\"errorCategory\":\"runtime\",\"source\":\"{}\","
            "\"line\":{},\"callDepth\":{}}}\n",
            sequence_, Trace::jsonEscape(funcName), escapedMessage, escapedMessage,
            Trace::jsonEscape(source != nullptr ? source : "?"), line, callDepth);
        ++sequence_;
    }

    std::ofstream file_;
    LuaState* state_ = nullptr;
    bool diffMode_ = false;
    u64 sequence_ = 0;
    StateRetirementTracker& retirements_;
    std::vector<StateIdentity> stateIdentities_;
    std::vector<OpenUpvalueEvidence> openUpvalues_;
    std::vector<LifecycleEvidence> pendingRetirementEvents_;
    std::vector<ExecutionStateEvidence> executionStates_;
    std::vector<StackTopProjection> stackTopProjections_;
    std::vector<InstructionEvidence> lastInstructions_;
};

std::string topError(lua_State* state) {
    const char* message = lua_tostring(state, -1);
    return message != nullptr ? message : "runtime error";
}

} // namespace

int main(int argc, char** argv) {
    const bool snapshotMode = argc == 4 && std::string(argv[1]) == "--trace";
    const bool diffMode = argc == 4 && std::string(argv[1]) == "--trace-diff";
    if (!snapshotMode && !diffMode) {
        std::cerr << "Usage: lua_vm_trace_evidence <--trace|--trace-diff> <trace.jsonl> <script.lua>\n";
        return 1;
    }

    try {
        StateRetirementTracker retirements;
        EngineContext engine(&StateRetirementTracker::allocate, &retirements);
        RuntimeServices services = engine.services();
        UPtr<LuaState> state = LuaState::create(services);
        if (!state) {
            std::cerr << "cannot create state: not enough memory\n";
            return 1;
        }
        StandardLibrary::openAll(state.get());

        lua_State* apiState = reinterpret_cast<lua_State*>(state.get());
        VmEvidenceJsonSink sink(argv[2], state.get(), diffMode, retirements);
        if (!sink.isOpen()) {
            std::cerr << "[TRACE] Warning: cannot open trace file: " << argv[2] << "\n";
            return 1;
        }

        VM::setTraceDiffEnabled(services, diffMode);
        VM::setTraceSink(services, &sink);
        std::cout << (diffMode ? "[TRACE] Trace diff enabled → " : "[TRACE] Trace enabled → ") << argv[2]
                  << '\n';

        int status = luaL_loadfile(apiState, argv[3]);
        if (status == 0) {
            status = lua_pcall(apiState, 0, LUA_MULTRET, 0);
        }
        if (status != 0) {
            const std::string message = topError(apiState);
            sink.emitTopLevelError(message);
            std::cerr << message << '\n';
        }

        sink.flush();
        VM::setTraceSink(services, nullptr);
        VM::setTraceDiffEnabled(services, false);
        std::cout << "[TRACE] Trace complete: " << sink.eventCount() << " events written.\n";
        return status == 0 ? 0 : 1;
    } catch (const std::exception& error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
