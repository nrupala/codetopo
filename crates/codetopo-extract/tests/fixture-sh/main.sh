#!/bin/sh
# Entry point.

. "core/engine.sh"

main() {
    local size
    size=$(clamp_size "$1")
    render "$size"
}

main "$@"
