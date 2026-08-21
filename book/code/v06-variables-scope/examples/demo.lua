local left, right = 10, 20
left, right = right, left

do
    local left = 99
    shadow_seen = left
end

return left * 100 + right
