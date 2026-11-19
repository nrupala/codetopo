using .Shapes: Circle
using .Core: Engine

state = Engine.EngineState(0)
wheel = Circle(2.0)
println(Engine.render(state, wheel))
