local prototype = { answer = 40 }
local object = setmetatable({ extra = 2 }, { __index = prototype })
local callable = setmetatable({ value = 6 }, {
    __call = function(self, amount)
        return self.value * amount
    end
})
print(object.answer + object.extra, callable(7))
