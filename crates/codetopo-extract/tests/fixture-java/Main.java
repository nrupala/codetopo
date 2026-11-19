package quarry;

import quarry.core.Engine;
import quarry.shapes.Circle;

public class Main {
    public static void main(String[] args) {
        Engine engine = new Engine();
        Circle wheel = new Circle(2.0);
        double area = engine.render(wheel);
        System.out.println(area);
    }
}
