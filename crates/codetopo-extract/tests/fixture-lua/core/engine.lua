local shapes = require("shapes")
local Shape = shapes.Shape

local Engine = {}
Engine.__index = Engine
Engine.renders = 0

function Engine.new()
  local self = setmetatable({}, Engine)
  return self
end

function Engine:render(wheel)
  Engine.renders = Engine.renders + 1
  return wheel:area()
end

return Engine
