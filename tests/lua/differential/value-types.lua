local values = {
    3,
    "text",
    true,
    {},
    function() end,
}
for i = 1, #values do
    io.write(type(values[i]), "\n")
end
io.write(type(nil), "\n")
io.write(type(1 + 2), ":", tostring(1 + 2), "\n")
