import { list_directory, run } from "memoized";
import { file_type } from "io";

const [dir] = ARGS ?? ["."]; 

for (const entry of list_directory(dir)) {
  if (file_type(entry) === "dir") {
    print(`${entry}/`);
    run("./test.js", entry);
  } else {
    print(entry);
  }
}
