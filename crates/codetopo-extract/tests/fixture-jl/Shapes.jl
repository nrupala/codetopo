module Shapes

abstract type Shape end

struct Circle <: Shape
    radius::Float64
end

area(c::Circle) = 3.141592653589793 * c.radius * c.radius

end
