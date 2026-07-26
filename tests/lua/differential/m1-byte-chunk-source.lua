local function emit(tag, payload)
    assert(string.len(tag) < 256)
    assert(string.len(payload) < 65536)
    local length = string.len(payload)
    io.write(
        string.char(string.len(tag)),
        tag,
        string.char(math.floor(length / 256), length % 256),
        payload
    )
end

local requested = "=" .. string.char(128, 255, 0, 88)
local chunk = assert(loadstring("return 1", requested))
local info = assert(debug.getinfo(chunk, "S"))

emit("requested", requested)
emit("source", info.source)
emit("short-src", info.short_src)
