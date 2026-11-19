#include "engine.h"

#include <stddef.h>

#define MAX_RENDER_DEPTH 4

int engine_count = 0;

static int clamp_size(int size) {
    if (size < 0) {
        return 0;
    }
    return size;
}

double engine_render(struct Engine *engine) {
    engine_count += 1;
    engine->size = clamp_size(engine->size);
    return circle_area(engine->wheel);
}

void engine_reset(void) {
    engine_count = 0;
}
