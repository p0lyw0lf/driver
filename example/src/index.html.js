import { write_output } from "io";

print("imported");
const [name] = ARGS;
print(`name ${name}`);

write_output(
  name,
  `
<!DOCTYPE html>
<html>
<head><title>Test Page</title></head>

<body>
  <h1>Hello, World!2</h1>
</body>
</html>
`,
);
print("wrote output");
