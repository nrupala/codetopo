#pragma once

#include "../shapes/Circle.hpp"

namespace quarry {

class Engine {
public:
    double render(const Circle& wheel);
    int renderCount() const;

private:
    int renders_ = 0;
};

} // namespace quarry
