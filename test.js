import { list_dir } from "driver";

for (const dir of list_dir("./query")) {
  console.log(dir);
}
