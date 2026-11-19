#!/bin/sh
# The engine: renders by delegating to the shape helper.

. "../lib/util.sh"
. "../lib/shapes.sh"

MAX_RENDERS=4
render_count=0

clamp_size() {
    if [ "$1" -lt 0 ]; then
        echo 0
    else
        echo "$1"
    fi
}

render() {
    render_count=$((render_count + 1))
    log_info "render $render_count"
    shape_area "$1"
}
