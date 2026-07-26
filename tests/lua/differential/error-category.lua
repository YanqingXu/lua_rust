local ok, err = pcall(function()
    return 1 + {}
end)
local category = "other"
if type(err) == "string" and string.find(err, "arithmetic", 1, true) then
    category = "arithmetic"
end
io.write(tostring(ok), ":", type(err), ":", category, "\n")
