#pragma once

#include "Shape.hpp"

namespace quarry {

class Circle : public Shape {
public:
    explicit Circle(double radius);
    double area() const override;

private:
    double radius_;
};

} // namespace quarry
