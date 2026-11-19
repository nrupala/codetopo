library(stats)

render_count <- 0

clamp_size <- function(size) {
  if (size < 0) {
    0
  } else {
    size
  }
}

render <- function(engine) {
  render_count <<- render_count + 1
  circle_area(engine$wheel)
}
