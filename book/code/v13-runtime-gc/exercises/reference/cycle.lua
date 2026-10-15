gc_collect()
local baseline = gc_live()

do
    local left, right = {}, {}
    left.other = right
    right.other = left
end
local first = gc_collect()
local unrooted_reclaimed = first >= 2 and gc_live() == baseline

do
    local left, right = {}, {}
    left.other = right
    right.other = left
    global_root = left
end
gc_collect()
local retained
do
    retained = gc_live() == baseline + 2
        and global_root.other.other == global_root
end

global_root = nil
local second = gc_collect()
return unrooted_reclaimed and retained
    and second >= 2 and gc_live() == baseline
