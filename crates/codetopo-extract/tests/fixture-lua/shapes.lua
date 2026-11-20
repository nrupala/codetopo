local Shape = {}
Shape.__index = Shape

function Shape.new(radius)
  local self = setmetatable({}, Shape)
  self.radius = radius
  return self
end

function Shape:area()
  return self.radius * self.radius
end

local Circle = {}
setmetatable(Circle, { __index = Shape })

function Circle:area()
  if self.radius > 1 then
    return Shape.area(self)
  end
  return 0
end

return { Shape = Shape, Circle = Circle }
