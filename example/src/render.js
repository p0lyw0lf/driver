/**
 * Tag template function for convenience. Should probably be written in Rust instead but ah well I love self-hosted cores.
 *
 * USAGE:
 * ```
 * const x = html`foo${false}bar${undefined}baz${null}qux`;
 * assert(x === "foobarbazqux");
 * ```
 *
 * That is, any naively-falsey value will be removed entirely, except for the digit zero. Arrays will be automatically flattened.
 */
export const html = (strings, ...exprs) => {
  const output = [];
  for (let i = 0; i / 2 < strings.length; i += 1) {
    if (i % 2 === 0) {
      output.push(strings[i / 2]);
    } else {
      const expr = exprs[(i - 1) / 2];
      output.push(asPrintable(expr));
    }
  }
  return output.join("");
};

/**
 * @returns string
 */
const asPrintable = (value) => {
  if (value === false || value === undefined || value === null) {
    return "";
  }
  if (Array.isArray(value)) {
    return value.map(asPrintable).join("");
  }
  return String(value);
};
