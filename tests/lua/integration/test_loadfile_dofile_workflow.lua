print("=== Integration: Loadfile + Dofile Workflow ===")

integration_helper_runs = 0
integration_helper_message = nil
integration_helper_join = nil
integration_helper_table = nil

local compiled, err = loadfile("tests/lua/integration/helper_workflow_chunk.lua")
assert(compiled ~= nil, err)
assert(type(compiled) == "function", "loadfile should return a function")

compiled()

assert(integration_helper_runs == 1, "helper run count after loadfile")
assert(integration_helper_message == "helper-run-1", "helper message after loadfile")

local joined1, fused1 = integration_helper_join("left", "right")
assert(joined1 == "left:right", "joined text")
assert(fused1 == "leftright", "fused text")

local a1, b1, c1 = unpack(integration_helper_table, 1, integration_helper_table.n)
assert(a1 == 1 and b1 == 11 and c1 == 21, "helper table contents")

dofile("tests/lua/integration/helper_workflow_chunk.lua")

assert(integration_helper_runs == 2, "helper run count after dofile")
assert(integration_helper_message == "helper-run-2", "helper message after dofile")

local dynamicEnv = { seed = 7 }
setmetatable(dynamicEnv, { __index = _G })

local chunk = assert(loadstring(
    "dynamic_value = seed * 6\n" ..
    "dynamic_text = integration_helper_message\n"
))

setfenv(chunk, dynamicEnv)
chunk()

assert(dynamicEnv.dynamic_value == 42, "dynamic value")
assert(dynamicEnv.dynamic_text == "helper-run-2", "dynamic text")

local ok, loadErr = pcall(function()
    local bad = assert(loadfile("tests/lua/integration/does_not_exist.lua"))
    return bad
end)

assert(not ok, "missing file should raise inside pcall")
assert(type(loadErr) == "string", "missing file error should be string")

print("helperRuns =", integration_helper_runs)
print("dynamicValue =", dynamicEnv.dynamic_value)
print("helperMessage =", integration_helper_message)
print("=== Loadfile workflow passed ===")
