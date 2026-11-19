#ifndef QUARRY_SHAPES_CIRCLE_H
#define QUARRY_SHAPES_CIRCLE_H

#include "shape.h"

struct Circle {
    double radius;
};

double circle_area(const struct Circle *circle);

#endif
