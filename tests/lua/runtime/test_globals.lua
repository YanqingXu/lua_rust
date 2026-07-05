-- Test script for comparing global variables between official Lua and lua/

print("=== Global Variables Test ===")

-- Test _VERSION
print("_VERSION:")
print(_VERSION)
print(type(_VERSION))

-- Test _PROMPT (should be nil in non-REPL mode)
print("_PROMPT:")
print(_PROMPT)
print(type(_PROMPT))

-- Test exit
print("exit:")
print(exit)
print(type(exit))

print("=== End Test ===")

