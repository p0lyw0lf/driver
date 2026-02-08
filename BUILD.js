import { read_file, list_directory, queue_task } from "memoized";
import { file_type } from "io";

/** @type {Array.<string>} */
const [dir] = ARGS ?? ["."];

for (const entry of list_directory(dir)) {
  if (file_type(entry) === "dir") {
    print(`${entry}/`);
    queue_task("./test.js", [entry]);
  } else {
    print(entry);
    read_file(entry); // ignore result, just want dependency
  }
}

export default "foobar";
