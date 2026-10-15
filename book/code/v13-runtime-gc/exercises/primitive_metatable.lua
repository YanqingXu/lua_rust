-- 只有 type_metatables 保留这个 metatable；局部变量已经离开作用域。
do
    local mt = { __index = { bonus = 42 } }
    settypemetatable('number', mt)
end
gc_collect()
local n = 1
return n.bonus
