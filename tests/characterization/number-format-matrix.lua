local values = {
    10/3, 1e-5, 1e-4, 1e13, 1e14, 1e20, 1e-308,
    4.9406564584124654e-324, 0, -1/math.huge, math.huge,
    -math.huge, math.sqrt(-1), 99999999999999.5,
    0.0000999999999999995, -math.sqrt(-1)
}

local file = assert(io.tmpfile())
for index, value in ipairs(values) do
    print(index, value, tostring(value), '' .. value,
          string.format('%s', value), string.format('%.14g', value))
    file:write(value, '\n')
end
file:seek('set', 0)
for index = 1, #values do
    print('io', index, file:read('*l'))
end

for _, value in ipairs(values) do
    local expected = tostring(value)
    assert(table.concat({value}) == expected)
    assert(string.gsub('a', 'a', value) == expected)
    assert(string.gsub('a', 'a', function() return value end) == expected)
    assert(string.gsub('a', 'a', {a = value}) == expected)
    assert(string.len(value) == #expected)
end
file:close()
print('number conversion routes passed')
