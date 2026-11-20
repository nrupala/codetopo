alias Quarry.Core.Engine
alias Quarry.Shapes.Circle

engine = Engine.new()
wheel = Circle.new(2.0)
IO.puts(Engine.render(engine, wheel))
