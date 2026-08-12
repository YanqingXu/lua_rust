cmake_minimum_required(VERSION 3.25)

project(lua_cpp_vm_trace_evidence LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT DEFINED LUA_CPP_SOURCE_ROOT OR LUA_CPP_SOURCE_ROOT STREQUAL "")
    message(FATAL_ERROR "LUA_CPP_SOURCE_ROOT must name the pinned lua_cpp checkout")
endif()
if(NOT DEFINED LUA_CPP_PACKAGE_ROOT OR LUA_CPP_PACKAGE_ROOT STREQUAL "")
    message(FATAL_ERROR "LUA_CPP_PACKAGE_ROOT must name the installed pinned lua_cpp package")
endif()
if(NOT DEFINED LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY OR LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY STREQUAL "")
    message(FATAL_ERROR "LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY must name the oracle executable directory")
endif()

find_package(LuaCpp CONFIG REQUIRED
    PATHS "${LUA_CPP_PACKAGE_ROOT}/lib/cmake/LuaCpp"
    NO_DEFAULT_PATH
)
add_executable(lua_vm_trace_evidence vm_trace_evidence_main.cpp)
target_link_libraries(lua_vm_trace_evidence PRIVATE LuaCpp::Lua)
target_include_directories(lua_vm_trace_evidence PRIVATE "${LUA_CPP_SOURCE_ROOT}/src")

if(MSVC)
    target_compile_options(lua_vm_trace_evidence PRIVATE /W4 /WX /permissive- /utf-8)
    target_link_options(lua_vm_trace_evidence PRIVATE /STACK:16777216)
else()
    target_compile_options(lua_vm_trace_evidence PRIVATE
        -Wall -Wextra -Wpedantic -Werror
    )
endif()

set_target_properties(lua_vm_trace_evidence PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY}"
    RUNTIME_OUTPUT_DIRECTORY_DEBUG "${LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY}"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY}"
    RUNTIME_OUTPUT_DIRECTORY_RELWITHDEBINFO "${LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY}"
    RUNTIME_OUTPUT_DIRECTORY_MINSIZEREL "${LUA_CPP_EVIDENCE_OUTPUT_DIRECTORY}"
)
