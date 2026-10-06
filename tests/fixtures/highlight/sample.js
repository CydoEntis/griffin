// A sample for the highlight tests.
import { render } from "./render.js";

function greet(name) {
  return "hello " + name;
}

const view = <div className="greeting">{greet("you")}</div>;
render(view, 42);
