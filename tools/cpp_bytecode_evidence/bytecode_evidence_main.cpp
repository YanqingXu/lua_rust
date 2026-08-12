/**
 * Read-only byte-safe evidence adapter for the pinned lua_cpp oracle.
 *
 * This executable deliberately lives outside the oracle checkout. It links
 * the pristine lua_core build and serializes constants and local-variable data
 * exposed by Proto's public const accessors, so the project can audit the
 * adapter independently without changing the fixed oracle commit or its
 * lua_bytecode output.
 */

#include "compiler/codegen/codegen.hpp"
#include "compiler/parser/parser.hpp"
#include "core/function.hpp"
#include "core/gc_string.hpp"
#include "io/file_loader.hpp"
#include "runtime/runtime_services.hpp"

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <exception>
#include <iomanip>
#include <iostream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>

using namespace Lua;

namespace {

constexpr std::string_view kOracleCommit = "87c15e69ceb94eb74e28226ccbefb7e196635711";

Proto* compileScript(RuntimeServices& services, const std::string& scriptPath) {
    Str source = readWholeFile(scriptPath);
    Parser parser(source, services);
    auto parsed = parser.parse();
    if (!parsed) {
        throw parsed.error();
    }

    Chunk chunk = std::move(*parsed);
    CodeGenerator codegen(services);
    auto generated = codegen.tryGenerate(chunk, scriptPath);
    if (!generated) {
        throw generated.error();
    }
    return *generated;
}

void writeHex(std::ostream& out, std::string_view bytes) {
    constexpr char digits[] = "0123456789abcdef";
    for (const char value : bytes) {
        const auto byte = static_cast<unsigned char>(value);
        out << digits[byte >> 4U] << digits[byte & 0x0fU];
    }
}

void writeByteEnvelope(std::ostream& out, const GCString* value) {
    if (value == nullptr) {
        out << "null";
        return;
    }

    const std::string_view bytes = value->getData();
    out << "{\"byte_length\":" << bytes.size() << ",\"bytes\":\"";
    writeHex(out, bytes);
    out << "\",\"encoding\":\"hex\"}";
}

void writeConstant(std::ostream& out, const Value& value) {
    if (value.isNil()) {
        out << "{\"type\":\"nil\"}";
        return;
    }
    if (value.isBoolean()) {
        out << "{\"type\":\"boolean\",\"value\":"
            << (value.asBoolean() ? "true" : "false") << '}';
        return;
    }
    if (value.isNumber()) {
        static_assert(sizeof(LuaNumber) == sizeof(std::uint64_t));
        const LuaNumber number = value.asNumber();
        std::uint64_t bits = 0;
        std::memcpy(&bits, &number, sizeof(bits));
        out << "{\"type\":\"number\",\"bits\":\"" << std::hex << std::setfill('0')
            << std::setw(16) << bits << std::dec << std::setfill(' ') << "\"}";
        return;
    }
    if (value.isString()) {
        out << "{\"type\":\"string\",\"value\":";
        writeByteEnvelope(out, value.asString());
        out << '}';
        return;
    }
    throw std::runtime_error("pinned oracle Proto contains an unsupported constant type");
}

void writeProto(std::ostream& out, const Proto* proto, const std::string& path, bool& first) {
    if (!first) {
        out << ',';
    }
    first = false;

    out << "{\"path\":\"" << path << "\",\"constants\":[";
    for (usize index = 0; index < proto->getConstantCount(); ++index) {
        if (index != 0) {
            out << ',';
        }
        writeConstant(out, proto->getConstant(index));
    }
    out << "],\"local_names\":[";
    for (usize index = 0; index < proto->getLocVarCount(); ++index) {
        if (index != 0) {
            out << ',';
        }
        writeByteEnvelope(out, proto->getLocVar(index).varname);
    }
    out << "],\"locals\":[";
    for (usize index = 0; index < proto->getLocVarCount(); ++index) {
        if (index != 0) {
            out << ',';
        }
        const LocVar& local = proto->getLocVar(index);
        out << "{\"name\":";
        writeByteEnvelope(out, local.varname);
        out << ",\"start_pc\":" << local.startpc << ",\"end_pc\":" << local.endpc
            << ",\"register\":" << local.reg << '}';
    }
    out << "]}";

    for (usize index = 0; index < proto->getSubProtoCount(); ++index) {
        const Proto* child = proto->getSubProto(index);
        if (child == nullptr) {
            throw std::runtime_error("pinned oracle Proto graph contains a null child");
        }
        writeProto(out, child, path + ".children[" + std::to_string(index) + "]", first);
    }
}

void writeDocument(std::ostream& out, const Proto* root) {
    out << "{\"schema_version\":2,\"oracle_commit\":\"" << kOracleCommit
        << "\",\"protos\":[";
    bool first = true;
    writeProto(out, root, "0", first);
    out << "]}\n";
}

} // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::cerr << "Usage: lua_bytecode_evidence <script.lua>\n";
        return 1;
    }

    try {
        EngineContext engine;
        RuntimeServices services = engine.services();
        writeDocument(std::cout, compileScript(services, argv[1]));
        return 0;
    } catch (const std::exception& error) {
        std::cerr << "[ERROR] Exception: " << error.what() << '\n';
        return 3;
    } catch (...) {
        std::cerr << "[ERROR] Unknown exception\n";
        return 4;
    }
}
