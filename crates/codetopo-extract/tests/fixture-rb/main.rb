require_relative "core/engine"

engine = Quarry::Engine.new
wheel = Quarry::Circle.new(2.0)
puts engine.render(wheel)
