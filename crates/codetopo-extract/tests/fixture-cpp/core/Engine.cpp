#include "Engine.hpp"

#include <cstddef>

namespace quarry {

double Engine::render(const Circle& wheel) {
    renders_ += 1;
    return wheel.area();
}

int Engine::renderCount() const {
    return renders_;
}

} // namespace quarry
