local function dirname(path)
    local dir = string.match(path or "", "^(.*)[/\\][^/\\]*$")
    if dir == nil or dir == "" then
        return "."
    end
    return string.gsub(dir, "\\", "/")
end

local function currentScriptPath()
    if arg and arg[0] and arg[0] ~= "" then
        return arg[0]
    end

    local info = debug and debug.getinfo and debug.getinfo(1, "S")
    local source = info and info.source
    if type(source) == "string" and string.sub(source, 1, 1) == "@" then
        return string.sub(source, 2)
    end

    return nil
end

local scriptDir = dirname(currentScriptPath())

local function fileExists(path)
    local file = io.open(path, "rb")
    if file then
        file:close()
        return true
    end
    return false
end

local function addPackagePath(pattern)
    if not string.find(package.path, pattern, 1, true) then
        package.path = pattern .. ";" .. package.path
    end
end

addPackagePath(scriptDir .. "/?.lua")
addPackagePath(scriptDir .. "/?/init.lua")

if not fileExists(scriptDir .. "/refactored/init.lua") then
    local function flatLoader(name)
        return function()
            return assert(loadfile(scriptDir .. "/" .. name .. ".lua"))()
        end
    end

    package.preload["refactored"] = function()
        return assert(loadfile(scriptDir .. "/init.lua"))()
    end

    package.preload["refactored.constants"] = flatLoader("constants")
    package.preload["refactored.tracer"] = flatLoader("tracer")
    package.preload["refactored.scheduler"] = flatLoader("scheduler")
    package.preload["refactored.graph"] = flatLoader("graph")
    package.preload["refactored.engine"] = flatLoader("engine")
    package.preload["refactored.primitives"] = flatLoader("primitives")
end

local s = require("refactored")

-- 开启追踪（这是你最强大的学习工具）
s.setTraceHandler(s.tracer.consoleHandler())

local count = s.signal(0, "count")
local doubled = s.computed(function() 
    return count() * 2 
end, "doubled")
local stop = s.effect(function() 
    print("count =", count(), "doubled =", doubled()) 
end, "logger")

print("----------------------  Signal Changed  ----------------------")
count(5)
count(10)
