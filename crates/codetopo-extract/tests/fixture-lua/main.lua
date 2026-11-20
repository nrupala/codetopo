local engine = require("core.engine")
local shapes = require("shapes")

sharedCount = 0

local wheel = shapes.Circle
local state = engine.new()
print(state:render(wheel))
print(sharedCount)

local function bump()
  sharedCount = sharedCount + 1
  return sharedCount
end

print(bump())
