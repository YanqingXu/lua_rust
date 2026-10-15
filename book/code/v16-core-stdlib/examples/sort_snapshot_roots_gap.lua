-- Known v0.16 failure, not a learner TODO or a production fix.
-- make_source's frame ends before sort, so its temporary item handles are not roots.
local function make_source()
  return { { rank = 4 }, { rank = 3 }, { rank = 2 }, { rank = 1 } }
end
local source = make_source()
local cleared = false
table.sort(source, function(left, right)
  if not cleared then
    cleared = true
    for index = 1, 4 do source[index] = nil end
    gc_collect()
  end
  return left.rank < right.rank
end)
return source[1].rank == 1 and source[4].rank == 4
