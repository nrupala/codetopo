#ifndef QUARRY_SHAPES_SHAPE_H
#define QUARRY_SHAPES_SHAPE_H

struct Shape {
    double (*area)(const struct Shape *self);
    int sides;
};

double shape_area(const struct Shape *shape);

#endif
