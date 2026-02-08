import { list_directory, queue_task } from "memoized";
import { file_type } from "io";

const root = "./src/";
/** @type {Array.<string>} */
const [to_build] = ARGS ?? [root];

if (file_type(to_build) === "dir") {
  // Look for all files ending with .js in all subdirectories
  for (const entry of list_directory(to_build)) {
    if (file_type(entry) === "dir") {
      queue_task("./BUILD.js", [entry]);
    } else if (entry.endsWith(".js")) {
      queue_task("./BUILD.js", [entry]);
    }
  }
} else if (to_build.endsWith(".js")) {
  // Execute the file to build (assuming it's javascript)
  queue_task(to_build, [to_build.slice(root.length, -3)]);
}
