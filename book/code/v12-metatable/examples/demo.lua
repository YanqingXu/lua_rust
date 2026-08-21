function vector(x, y)
    local methods = {}
    local mt = {}

    mt.__index = methods
    mt.__add = function(left, right)
        return vector(left.x + right.x, left.y + right.y)
    end
    mt.__len = function(self)
        return self.x * self.x + self.y * self.y
    end
    mt.__call = function(self, scale)
        return vector(self.x * scale, self.y * scale)
    end
    mt.__tostring = function(self)
        return "vector(" .. self.x .. "," .. self.y .. ")"
    end

    methods.sum = function(self)
        return self.x + self.y
    end

    return setmetatable({ x = x, y = y }, mt)
end

local value = (vector(1, 2) + vector(3, 4))(2)
local label_ok = tostring(value) == "vector(8,12)"

return value.x * 1000 + value.y * 10 + #value
    + (label_ok and value:sum() or 0)
