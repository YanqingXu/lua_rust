demo_module_loads = (demo_module_loads or 0) + 1

local base = 1700
return {
    finish = function(content_length)
        return base + content_length + 10
    end,
}
