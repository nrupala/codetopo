#include "Circle.hpp"

namespace quarry {

Circle::Circle(double radius) : radius_(radius) {}

double Circle::area() const {
    return 3.141592653589793 * radius_ * radius_;
}

} // namespace quarry
