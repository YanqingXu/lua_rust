package.path = "book/code/v17-host-stdlib-modules/examples/?.lua"
local lesson = require("demo_module")
assert(require("demo_module") == lesson)

local path = os.tmpname()
local file = assert(io.open(path, "w+"))
file:write("Lua 5.1")
file:seek("set", 0)
local content = file:read("*a")
file:close()
assert(os.remove(path))

local info = debug.getinfo(lesson.finish)
return (info.what == "Lua") and lesson.finish(#content) or 0
