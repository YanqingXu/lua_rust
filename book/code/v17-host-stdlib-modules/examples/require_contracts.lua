-- Predict cache identity, the cycle's temporary value, and a failed load's retry.
local loads = 0
package.preload.lesson = function(name)
    loads = loads + 1
    return { answer = 42, name = name }, 'discarded'
end
local first = require('lesson')
local second = require('lesson')
assert(first == second and first.answer == 42 and loads == 1)

package.preload.cycle_a = function()
    return { b = require('cycle_b') }
end
package.preload.cycle_b = function()
    return { saw_temporary_true = require('cycle_a') == true }
end
local cycle = require('cycle_a')
assert(cycle.b.saw_temporary_true and package.loaded.cycle_a == cycle)

local attempts, effects = 0, 0
package.preload.retry_lesson = function()
    attempts = attempts + 1
    effects = effects + 1
    if attempts == 1 then error('first load fails') end
    return { answer = 42 }
end
local ok = pcall(require, 'retry_lesson')
assert(not ok and package.loaded.retry_lesson == nil)
local retried = require('retry_lesson')
return retried.answer == 42 and attempts == 2 and effects == 2
