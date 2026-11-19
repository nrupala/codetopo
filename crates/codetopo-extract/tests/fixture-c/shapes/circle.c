#include "circle.h"

#include <math.h>

static const double PI = 3.141592653589793;

double circle_area(const struct Circle *circle) {
    return PI * circle->radius * circle->radius;
}
