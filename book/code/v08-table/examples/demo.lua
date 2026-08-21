local inventory = {3, 5, name = "bolts"}
local alias = inventory

alias[2] = 7
inventory.extra = 10
inventory.self = inventory

return inventory[1] + inventory[2] + inventory.extra
