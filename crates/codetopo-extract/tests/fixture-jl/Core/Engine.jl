module Engine

using ..Shapes: Circle

mutable struct EngineState
    renders::Int
end

function render(state::EngineState, wheel::Circle)
    state.renders += 1
    return area(wheel)
end

end
