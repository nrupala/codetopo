package quarry.core;

import java.util.List;

import quarry.shapes.Circle;

public class Engine {
    private int renders;

    public double render(Circle wheel) {
        renders += 1;
        return wheel.area();
    }

    public int renderCount() {
        return renders;
    }
}
