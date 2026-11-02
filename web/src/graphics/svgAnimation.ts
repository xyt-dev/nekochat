// SPDX-License-Identifier: Apache-2.0 OR MIT
/** Walk CSS component values without treating quoted/commented delimiters as syntax. */
function escapeEnd(value: string, start: number): number {
  let end = start + 1;
  if (/[\da-f]/i.test(value[end] ?? "")) {
    let count = 0;
    while (count < 6 && /[\da-f]/i.test(value[end] ?? "")) { end++; count++; }
    if (value.startsWith("\r\n", end)) end += 2;
    else if (/[\t\n\f\r ]/.test(value[end] ?? "")) end++;
    return end;
  }
  return Math.min(value.length, end + 1);
}
function decodeIdentifier(value: string): string {
  return value.replace(/\\([\da-f]{1,6})(?:\r\n|[\t\n\f\r ])?|\\([^\n\r\f])/gi, (_, hex: string | undefined, character: string | undefined) => {
    if (!hex) return character ?? "";
    const code = Number.parseInt(hex, 16);
    return code === 0 || code > 0x10ffff || code >= 0xd800 && code <= 0xdfff ? "\ufffd" : String.fromCodePoint(code);
  });
}
function tokenEnd(value: string, start: number): number {
  if (value.startsWith("/*", start)) {
    const end = value.indexOf("*/", start + 2);
    return end < 0 ? value.length : end + 2;
  }
  const quote = value[start];
  if (quote === '"' || quote === "'") {
    let end = start + 1;
    while (end < value.length && value[end] !== quote) end = value[end] === "\\" ? escapeEnd(value, end) : end + 1;
    return Math.min(value.length, end + 1);
  }
  return start;
}
function triviaEnd(value: string, start: number): number {
  let end = start;
  while (end < value.length) {
    if (value.startsWith("/*", end)) end = tokenEnd(value, end);
    else if (/[\t\n\f\r ]/.test(value[end])) end++;
    else break;
  }
  return end;
}
function identifierEnd(value: string, start: number): number {
  let end = start;
  while (end < value.length) {
    if (value[end] === "\\") end = escapeEnd(value, end);
    else if (/[\w-]|[^\x00-\x7f]/.test(value[end])) end++;
    else break;
  }
  return end;
}
function closingParenthesis(value: string, start: number): number {
  let depth = 1;
  for (let index = start; index < value.length;) {
    const end = tokenEnd(value, index);
    if (end !== index) { index = end; continue; }
    if (value[index] === "\\") { index = escapeEnd(value, index); continue; }
    if (value[index] === "(") depth++;
    if (value[index] === ")" && --depth === 0) return index;
    index++;
  }
  return value.length;
}
/** Substitute known CSS tokens; native SMIL contexts can also resolve every fallback. */
export function bindSvgAnimationVariables(value: string, properties: Readonly<Record<string, string>>, resolveFallbacks = false): string {
  let result = "";
  for (let index = 0; index < value.length;) {
    const opaqueEnd = tokenEnd(value, index);
    if (opaqueEnd !== index) { result += value.slice(index, opaqueEnd); index = opaqueEnd; continue; }
    const end = identifierEnd(value, index);
    if (end === index) { result += value[index++]; continue; }
    if (decodeIdentifier(value.slice(index, end)).toLowerCase() !== "var" || value[end] !== "(") {
      result += value.slice(index, end); index = end; continue;
    }
    const close = closingParenthesis(value, end + 1);
    if (close === value.length) { result += value.slice(index); break; }
    const contents = value.slice(end + 1, close);
    // Read the identifier as a token so escapes and comments do not become delimiters.
    const nameStart = triviaEnd(contents, 0);
    const nameEnd = identifierEnd(contents, nameStart);
    const name = decodeIdentifier(contents.slice(nameStart, nameEnd));
    const tail = contents.slice(triviaEnd(contents, nameEnd));
    const replacement = Object.hasOwn(properties, name) ? properties[name] : undefined;
    if (replacement) result += replacement;
    else if ((resolveFallbacks || name.startsWith("--nk-")) && tail.startsWith(",")) result += bindSvgAnimationVariables(tail.slice(1).trim(), properties, resolveFallbacks);
    else result += `${value.slice(index, end + 1)}${bindSvgAnimationVariables(contents, properties, resolveFallbacks)})`;
    index = close + 1;
  }
  return result;
}
/** SMIL values use top-level semicolons, unlike CSS functions and string tokens. */
export function svgAnimationValues(value: string): string[] {
  const values: string[] = [];
  let depth = 0, start = 0;
  for (let index = 0; index < value.length;) {
    const end = tokenEnd(value, index);
    if (end !== index) { index = end; continue; }
    if (value[index] === "\\") { index = escapeEnd(value, index); continue; }
    if (value[index] === "(") depth++;
    else if (value[index] === ")") depth--;
    else if (value[index] === ";" && depth === 0) { values.push(value.slice(start, index)); start = index + 1; }
    index++;
  }
  values.push(value.slice(start));
  return values;
}
