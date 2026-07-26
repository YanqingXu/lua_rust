local observed = setmetatable({}, {__mode = "v"})
do
    local value = {marker = true}
    observed[1] = value
end
collectgarbage()
collectgarbage()
io.write(observed[1] == nil and "collected\n" or "retained\n")
