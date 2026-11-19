#!/bin/sh
# A shape helper in shell form.

shape_area() {
    local radius="$1"
    echo "3.141592653589793 * $radius * $radius" | bc
}
