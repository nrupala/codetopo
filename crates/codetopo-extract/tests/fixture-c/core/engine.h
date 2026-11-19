#ifndef QUARRY_CORE_ENGINE_H
#define QUARRY_CORE_ENGINE_H

#include "../shapes/circle.h"

struct Engine {
    struct Circle *wheel;
    int size;
};

extern int engine_count;

double engine_render(struct Engine *engine);
void engine_reset(void);

#endif
