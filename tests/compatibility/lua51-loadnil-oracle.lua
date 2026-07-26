local probes = {
  function ()
    local a,b,c
    local d; local e;
    a = nil; d=nil
  end,
  function () repeat local x = 1 until false end,
  function () repeat local x until nil end,
}
return probes
