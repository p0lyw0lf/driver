import { STATIC_URL } from "../data/urls.js";
import { html } from "../render.js";

/**
 * @typedef Props
 * @type {object}
 * @property {string} src - Relative to https://static.wolfgirl.dev/pfps/
 * @property {string} alt - Alt text for the image.
 */

/**
 * @param {Props} props
 * @returns string
 */
export const HeaderPfp = ({ src, alt }) => {
  return html`<a href="/art/">
    <img
      src="${new URL(`/pfps/${src}`, STATIC_URL)}"
      width="128"
      height="128"
      loading="eager"
      alt="${alt}"
    />
  </a>`;
};
