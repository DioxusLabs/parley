// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Runs inside a loaded WPT page (via WebDriver `execute`) and describes every block
// container whose content is purely inline text: the inputs Parley needs to lay the
// same text out (content-box size, the container's text properties, one styled run per
// text node) and Chrome's per-character client rects as the expected output.
//
// This is a plain script, not a module: WebDriver wraps it in a function body, so
// `return` at the top level is what hands the result back.

/** Container properties that shape line layout. */
const CONTAINER_PROPERTIES = [
  "direction",
  "unicode-bidi",
  "writing-mode",
  "text-orientation",
  "text-align",
  "text-align-last",
  "text-justify",
  "text-indent",
  "white-space-collapse",
  "text-wrap-mode",
  "text-wrap-style",
  "word-break",
  "overflow-wrap",
  "line-break",
  "hyphens",
  "tab-size",
  "text-autospace",
  "text-spacing-trim",
  "hanging-punctuation",
  "text-box-trim",
  "text-box-edge",
  "text-fit",
  "text-grow",
  "text-shrink",
  "text-emphasis-style",
  "initial-letter",
  "font-family",
  "font-size",
  "font-weight",
  "font-style",
  "font-stretch",
  "font-kerning",
  "font-variant-ligatures",
  "font-variant-caps",
  "font-variant-numeric",
  "font-variant-east-asian",
  "font-feature-settings",
  "font-variation-settings",
  "line-height",
  "letter-spacing",
  "word-spacing",
  "text-transform",
  "text-combine-upright",
  "vertical-align",
];

/**
 * Per-run (text node) properties, read from the node's parent element. Only values
 * differing from the container's end up in the fixture.
 */
const RUN_PROPERTIES = [
  "direction",
  "unicode-bidi",
  "white-space-collapse",
  "text-wrap-mode",
  "word-break",
  "overflow-wrap",
  "line-break",
  "hyphens",
  "font-family",
  "font-size",
  "font-weight",
  "font-style",
  "font-stretch",
  "font-kerning",
  "font-variant-ligatures",
  "font-variant-caps",
  "font-variant-numeric",
  "font-variant-east-asian",
  "font-feature-settings",
  "font-variation-settings",
  "line-height",
  "letter-spacing",
  "word-spacing",
  "text-transform",
  "vertical-align",
  "text-combine-upright",
  "text-emphasis-style",
  "text-justify",
  "initial-letter",
];

/** Display values that establish a block container with an inline formatting context. */
const BLOCK_CONTAINER_DISPLAYS = new Set([
  "block",
  "inline-block",
  "list-item",
  "table-cell",
  "table-caption",
  "flow-root",
]);

/** Element names whose content is never plain inline text. */
const REJECTED_INLINE_ELEMENTS = new Set([
  "WBR",
  "IMG",
  "VIDEO",
  "CANVAS",
  "SVG",
  "MATH",
  "INPUT",
  "TEXTAREA",
  "SELECT",
  "BUTTON",
  "IFRAME",
  "OBJECT",
  "EMBED",
  "RUBY",
  "RT",
  "RP",
]);

function readProperties(style, names) {
  const out = {};
  for (const name of names) {
    out[name] = style.getPropertyValue(name);
  }
  return out;
}

function isZeroLength(value) {
  return value === "" || value === "0px" || value === "none" || value === "auto";
}

function hasGeneratedContent(element) {
  for (const pseudo of ["::before", "::after", "::marker"]) {
    const style = getComputedStyle(element, pseudo);
    if (style.content !== "none" && style.content !== "normal" && style.content !== "") {
      return true;
    }
  }
  return false;
}

/**
 * Checks that `element`'s subtree is inline text only, pushing reasons to `problems`.
 * Nested inline elements are fine as long as they contribute no box of their own
 * beyond the text (no padding, border, margin, non-baseline vertical-align).
 */
function checkInlineContent(element, problems, depth) {
  for (const child of element.childNodes) {
    if (child.nodeType === Node.TEXT_NODE || child.nodeType === Node.COMMENT_NODE) {
      continue;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) {
      problems.push(`unexpected node type ${child.nodeType}`);
      continue;
    }
    const tag = child.tagName.toUpperCase();
    if (REJECTED_INLINE_ELEMENTS.has(tag)) {
      problems.push(`contains <${tag.toLowerCase()}>`);
      continue;
    }
    const style = getComputedStyle(child);
    if (style.display === "none") {
      continue;
    }
    if (tag === "BR") {
      // A forced break: becomes a preserved newline run.
      continue;
    }
    if (style.display === "contents") {
      checkInlineContent(child, problems, depth + 1);
      continue;
    }
    if (style.display !== "inline") {
      problems.push(`contains display:${style.display} <${tag.toLowerCase()}>`);
      continue;
    }
    if (style.float !== "none") {
      problems.push("contains a float");
    }
    if (style.position === "absolute" || style.position === "fixed") {
      problems.push("contains an out-of-flow element");
    } else if (style.position === "relative" &&
      !(isZeroLength(style.left) && isZeroLength(style.top) &&
        isZeroLength(style.right) && isZeroLength(style.bottom))) {
      problems.push("contains a relatively offset inline");
    }
    for (const side of ["left", "right"]) {
      if (!isZeroLength(style.getPropertyValue(`padding-${side}`)) ||
        !isZeroLength(style.getPropertyValue(`margin-${side}`)) ||
        (style.getPropertyValue(`border-${side}-style`) !== "none" &&
          !isZeroLength(style.getPropertyValue(`border-${side}-width`)))) {
        problems.push(`inline <${tag.toLowerCase()}> has horizontal padding/border/margin`);
        break;
      }
    }
    if (style.verticalAlign !== "baseline") {
      problems.push(`inline <${tag.toLowerCase()}> has vertical-align:${style.verticalAlign}`);
    }
    if (hasGeneratedContent(child)) {
      problems.push(`inline <${tag.toLowerCase()}> has generated content`);
    }
    checkInlineContent(child, problems, depth + 1);
  }
}

function hasText(element) {
  return textNodesOf(element).some((node) =>
    node.nodeType === Node.TEXT_NODE && node.data.trim() !== ""
  );
}

/**
 * Text nodes and `<br>` elements of `element` in document order, skipping
 * `display:none` subtrees.
 */
function textNodesOf(element) {
  const nodes = [];
  const visit = (node) => {
    for (const child of node.childNodes) {
      if (child.nodeType === Node.TEXT_NODE) {
        nodes.push(child);
      } else if (child.nodeType === Node.ELEMENT_NODE) {
        const style = getComputedStyle(child);
        if (style.display === "none") {
          continue;
        }
        if (child.tagName.toUpperCase() === "BR") {
          nodes.push(child);
        } else {
          visit(child);
        }
      }
    }
  };
  visit(element);
  return nodes;
}

function closestLang(element) {
  const withLang = element.closest("[lang]");
  return withLang === null ? "" : withLang.getAttribute("lang");
}

function cssPath(element) {
  const parts = [];
  let current = element;
  while (current !== null && current.nodeType === Node.ELEMENT_NODE) {
    let part = current.tagName.toLowerCase();
    if (current.id !== "") {
      parts.unshift(`${part}#${current.id}`);
      break;
    }
    const parent = current.parentElement;
    if (parent !== null) {
      const siblings = Array.from(parent.children).filter((c) => c.tagName === current.tagName);
      if (siblings.length > 1) {
        part += `:nth-of-type(${siblings.indexOf(current) + 1})`;
      }
    }
    parts.unshift(part);
    current = parent;
  }
  return parts.join(" > ");
}

/**
 * Client rects of every code point of `node`, relative to `(originX, originY)`.
 * A code point may have zero rects (collapsed white space) or several (rare splits).
 */
function characterRects(node, originX, originY) {
  const data = node.data;
  const chars = [];
  const range = document.createRange();
  let offset = 0;
  while (offset < data.length) {
    const code = data.codePointAt(offset);
    const length = code > 0xffff ? 2 : 1;
    range.setStart(node, offset);
    range.setEnd(node, offset + length);
    const rects = [];
    for (const rect of range.getClientRects()) {
      rects.push([
        rect.left - originX,
        rect.top - originY,
        rect.width,
        rect.height,
      ]);
    }
    chars.push(rects);
    offset += length;
  }
  return chars;
}

/**
 * Border boxes of every rendered float in the document. A float anywhere in the
 * document may shorten the line boxes of a block it overlaps, which the block's own
 * subtree gives no hint of.
 */
function floatRects() {
  const rects = [];
  for (const element of document.querySelectorAll("body *")) {
    if (getComputedStyle(element).float !== "none") {
      for (const rect of element.getClientRects()) {
        rects.push(rect);
      }
    }
  }
  return rects;
}

function intersects(a, b) {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

/**
 * The selectors of every `::first-line` / `::first-letter` rule, minus the pseudo
 * element. Those styles are invisible to `getComputedStyle`, so a block they apply to
 * (they inherit into nested blocks) cannot be described faithfully.
 */
function firstLineSelectors() {
  const selectors = [];
  for (const sheet of document.styleSheets) {
    let rules;
    try {
      rules = sheet.cssRules;
    } catch (_) {
      continue;
    }
    for (const rule of rules) {
      if (!(rule instanceof CSSStyleRule)) {
        continue;
      }
      for (const selector of rule.selectorText.split(",")) {
        const match = selector.match(/^(.*?)::?first-(line|letter)\s*$/);
        if (match !== null) {
          selectors.push(match[1].trim() === "" ? "*" : match[1].trim());
        }
      }
    }
  }
  return selectors;
}

function hasFirstLineStyle(element, selectors) {
  return selectors.some((selector) => {
    try {
      return element.closest(selector) !== null;
    } catch (_) {
      return true;
    }
  });
}

function describeBlock(element, index, floats, firstLine) {
  const problems = [];
  const style = getComputedStyle(element);
  if (hasGeneratedContent(element)) {
    problems.push("container has generated content");
  }
  if (hasFirstLineStyle(element, firstLine)) {
    problems.push("styled by a first-line or first-letter pseudo-element");
  }
  if (floats.some((float) => intersects(float, element.getBoundingClientRect()))) {
    problems.push("a float overlaps the block");
  }
  if (style.getPropertyValue("writing-mode") !== "horizontal-tb") {
    problems.push(`writing-mode:${style.getPropertyValue("writing-mode")}`);
  }
  if (style.display === "list-item" && style.listStyleType !== "none" &&
    style.listStylePosition === "inside") {
    problems.push("list item with an inside marker");
  }
  // Layout of the block's lines that its own and its runs' styles do not show.
  for (let ancestor = element; ancestor !== null; ancestor = ancestor.parentElement) {
    const ancestorStyle = getComputedStyle(ancestor);
    if (ancestorStyle.columnCount !== "auto" || ancestorStyle.columnWidth !== "auto") {
      problems.push("inside a multi-column container");
      break;
    }
    if (ancestor !== element && ancestorStyle.getPropertyValue("text-box-trim") !== "none") {
      problems.push("an ancestor has text-box-trim");
      break;
    }
  }
  checkInlineContent(element, problems, 0);

  const rect = element.getBoundingClientRect();
  const paddingLeft = parseFloat(style.paddingLeft);
  const paddingTop = parseFloat(style.paddingTop);
  const borderLeft = parseFloat(style.borderLeftWidth);
  const borderTop = parseFloat(style.borderTopWidth);
  const originX = rect.left + borderLeft + paddingLeft;
  const originY = rect.top + borderTop + paddingTop;
  const width = rect.width - borderLeft - paddingLeft -
    parseFloat(style.borderRightWidth) - parseFloat(style.paddingRight);
  const height = rect.height - borderTop - paddingTop - parseFloat(style.borderBottomWidth) - parseFloat(style.paddingBottom);

  const runs = [];
  for (const node of textNodesOf(element)) {
    if (node.nodeType === Node.ELEMENT_NODE) {
      // A `<br>`: a newline that is never collapsed. The line it ends takes its
      // metrics from the parent's style, not from styles set on the `<br>` itself
      // (see css-inline/br-font-size.html).
      const brStyle = readProperties(getComputedStyle(node.parentElement), RUN_PROPERTIES);
      brStyle["white-space-collapse"] = "preserve";
      runs.push({
        text: "\n",
        lang: closestLang(node),
        style: brStyle,
        chars: [[]],
      });
      continue;
    }
    const parent = node.parentElement;
    const parentStyle = getComputedStyle(parent);
    runs.push({
      text: node.data,
      lang: closestLang(parent),
      style: readProperties(parentStyle, RUN_PROPERTIES),
      chars: characterRects(node, originX, originY),
    });
  }

  return {
    index,
    path: cssPath(element),
    width,
    height,
    lang: closestLang(element),
    style: readProperties(style, CONTAINER_PROPERTIES),
    runs,
    problems,
  };
}

function fontFaces() {
  const faces = [];
  for (const sheet of document.styleSheets) {
    let rules;
    try {
      rules = sheet.cssRules;
    } catch (_) {
      continue;
    }
    const base = sheet.href === null ? document.baseURI : sheet.href;
    for (const rule of rules) {
      if (!(rule instanceof CSSFontFaceRule)) {
        continue;
      }
      const family = rule.style.getPropertyValue("font-family").replace(/^['"]|['"]$/g, "");
      const src = rule.style.getPropertyValue("src");
      const urls = [];
      for (const match of src.matchAll(/url\((['"]?)([^'")]*)\1\)/g)) {
        try {
          urls.push(new URL(match[2], base).pathname);
        } catch (_) {
          // An unresolvable URL never loaded anyway.
        }
      }
      faces.push({
        family,
        urls,
        weight: rule.style.getPropertyValue("font-weight"),
        style: rule.style.getPropertyValue("font-style"),
        stretch: rule.style.getPropertyValue("font-stretch"),
        unicodeRange: rule.style.getPropertyValue("unicode-range"),
      });
    }
  }
  return faces;
}

function collectBlocks() {
  const blocks = [];
  const floats = floatRects();
  const firstLine = firstLineSelectors();
  const all = document.querySelectorAll("body, body *");
  let index = 0;
  for (const element of all) {
    const style = getComputedStyle(element);
    if (element.namespaceURI !== "http://www.w3.org/1999/xhtml" ||
      !BLOCK_CONTAINER_DISPLAYS.has(style.display) || element.getClientRects().length === 0) {
      continue;
    }
    // Only containers whose in-flow content is inline: any block-level child means
    // this element establishes a block formatting context of blocks, not lines.
    let hasBlockChild = false;
    for (const child of element.children) {
      const childStyle = getComputedStyle(child);
      if (childStyle.display !== "none" && childStyle.display !== "inline" &&
        childStyle.display !== "contents" && childStyle.position !== "absolute" &&
        childStyle.position !== "fixed" && childStyle.float === "none") {
        hasBlockChild = true;
        break;
      }
    }
    if (hasBlockChild || !hasText(element)) {
      continue;
    }
    blocks.push(describeBlock(element, index, floats, firstLine));
    index += 1;
  }
  return blocks;
}

return JSON.stringify({
  url: location.href,
  title: document.title,
  viewport: { width: window.innerWidth, height: window.innerHeight },
  fonts: fontFaces(),
  blocks: collectBlocks(),
});
