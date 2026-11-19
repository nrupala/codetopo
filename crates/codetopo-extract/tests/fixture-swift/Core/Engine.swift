import Foundation

final class Engine {
    var renders = 0

    func render(wheel: Circle) -> Double {
        renders += 1
        return wheel.area()
    }
}
