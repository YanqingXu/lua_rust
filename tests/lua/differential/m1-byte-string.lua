local function emit(tag, payload)
    assert(string.len(tag) < 256)
    assert(string.len(payload) < 65536)
    local length = string.len(payload)
    io.write(
        string.char(string.len(tag)),
        tag,
        string.char(math.floor(length / 256), length % 256),
        payload
    )
end

local raw = string.char(0, 128, 255, 65, 66)
emit("raw", raw)

local transformed =
    string.reverse(raw) ..
    string.sub(raw, 2, 4) ..
    string.rep(string.char(0, 255), 2)
emit("byte-string", transformed)

local temporary = assert(io.tmpfile())
assert(temporary:write(raw))
assert(temporary:seek("set", 0) == 0)
local roundtrip = assert(temporary:read("*a"))
assert(temporary:close())
emit("tmpfile", roundtrip)

local module_name = string.char(128, 255, 77)
package.preload[module_name] = function(requested)
    return string.reverse(requested) .. string.char(127)
end
local loaded = require(module_name)
assert(require(module_name) == loaded)
emit("preload-name", loaded)

local globals = _G
local loaded_modules = package.loaded
local module_function = module
local module_first = string.char(255)
local module_leaf = string.char(128)
local nested_name = module_first .. "." .. module_leaf
local function create_nested_module(name)
    module_function(name)
    return loaded_modules[name]
end
local nested_module = create_nested_module(nested_name)
assert(nested_module == globals[module_first][module_leaf])
assert(nested_module._NAME == nested_name)
assert(nested_module._PACKAGE == module_first .. ".")
emit("module-name", nested_module._NAME .. nested_module._PACKAGE)
