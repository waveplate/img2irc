var __typeError = (msg) => {
  throw TypeError(msg);
};
var __accessCheck = (obj, member, msg) => member.has(obj) || __typeError("Cannot " + msg);
var __privateGet = (obj, member, getter) => (__accessCheck(obj, member, "read from private field"), getter ? getter.call(obj) : member.get(obj));
var __privateAdd = (obj, member, value) => member.has(obj) ? __typeError("Cannot add the same private member more than once") : member instanceof WeakSet ? member.add(obj) : member.set(obj, value);
var __privateSet = (obj, member, value, setter) => (__accessCheck(obj, member, "write to private field"), setter ? setter.call(obj, value) : member.set(obj, value), value);
var __privateMethod = (obj, member, method) => (__accessCheck(obj, member, "access private method"), method);

// ../irc2html/src/palettes.js
var IRC99_HEX = [
  16777215,
  0,
  127,
  37632,
  16711680,
  8323072,
  10223772,
  16547584,
  16776960,
  64512,
  37779,
  65535,
  252,
  16711935,
  5592405,
  11184810,
  4653056,
  4661504,
  4671232,
  3294976,
  18176,
  18220,
  18247,
  10055,
  71,
  3014727,
  4653127,
  4653098,
  7602176,
  7617024,
  7631872,
  5338112,
  29696,
  29769,
  29812,
  16500,
  116,
  4915316,
  7602292,
  7602245,
  11862016,
  11887360,
  11908352,
  8238336,
  46336,
  46449,
  46517,
  25525,
  181,
  7667893,
  11862197,
  11862123,
  16711680,
  16747520,
  16776960,
  11730688,
  65280,
  65440,
  65535,
  36095,
  255,
  10813695,
  16711935,
  16711832,
  16734553,
  16757849,
  16777073,
  13631328,
  7339887,
  6684617,
  7208959,
  5879039,
  5855743,
  12868095,
  16738047,
  16734652,
  16751772,
  16765852,
  16777116,
  14876572,
  10289052,
  10289115,
  10289151,
  10277887,
  10263807,
  14458111,
  16751871,
  16749779,
  0,
  1250067,
  2631720,
  3552822,
  5066061,
  6645093,
  8487297,
  10461087,
  12369084,
  14869218,
  16777215
];
var ANSI256_HEX = [
  0,
  8388608,
  32768,
  8421376,
  128,
  8388736,
  32896,
  12632256,
  8421504,
  16711680,
  65280,
  16776960,
  255,
  16711935,
  65535,
  16777215,
  0,
  95,
  135,
  175,
  215,
  255,
  24320,
  24415,
  24455,
  24495,
  24535,
  24575,
  34560,
  34655,
  34695,
  34735,
  34775,
  34815,
  44800,
  44895,
  44935,
  44975,
  45015,
  45055,
  55040,
  55135,
  55175,
  55215,
  55255,
  55295,
  65280,
  65375,
  65415,
  65455,
  65495,
  65535,
  6225920,
  6226015,
  6226055,
  6226095,
  6226135,
  6226175,
  6250240,
  6250335,
  6250375,
  6250415,
  6250455,
  6250495,
  6260480,
  6260575,
  6260615,
  6260655,
  6260695,
  6260735,
  6270720,
  6270815,
  6270855,
  6270895,
  6270935,
  6270975,
  6280960,
  6281055,
  6281095,
  6281135,
  6281175,
  6281215,
  6291200,
  6291295,
  6291335,
  6291375,
  6291415,
  6291455,
  8847360,
  8847455,
  8847495,
  8847535,
  8847575,
  8847615,
  8871680,
  8871775,
  8871815,
  8871855,
  8871895,
  8871935,
  8881920,
  8882015,
  8882055,
  8882095,
  8882135,
  8882175,
  8892160,
  8892255,
  8892295,
  8892335,
  8892375,
  8892415,
  8902400,
  8902495,
  8902535,
  8902575,
  8902615,
  8902655,
  8912640,
  8912735,
  8912775,
  8912815,
  8912855,
  8912895,
  11468800,
  11468895,
  11468935,
  11468975,
  11469015,
  11469055,
  11493120,
  11493215,
  11493255,
  11493295,
  11493335,
  11493375,
  11503360,
  11503455,
  11503495,
  11503535,
  11503575,
  11503615,
  11513600,
  11513695,
  11513735,
  11513775,
  11513815,
  11513855,
  11523840,
  11523935,
  11523975,
  11524015,
  11524055,
  11524095,
  11534080,
  11534175,
  11534215,
  11534255,
  11534295,
  11534335,
  14090240,
  14090335,
  14090375,
  14090415,
  14090455,
  14090495,
  14114560,
  14114655,
  14114695,
  14114735,
  14114775,
  14114815,
  14124800,
  14124895,
  14124935,
  14124975,
  14125015,
  14125055,
  14135040,
  14135135,
  14135175,
  14135215,
  14135255,
  14135295,
  14145280,
  14145375,
  14145415,
  14145455,
  14145495,
  14145535,
  14155520,
  14155615,
  14155655,
  14155695,
  14155735,
  14155775,
  16711680,
  16711775,
  16711815,
  16711855,
  16711895,
  16711935,
  16736e3,
  16736095,
  16736135,
  16736175,
  16736215,
  16736255,
  16746240,
  16746335,
  16746375,
  16746415,
  16746455,
  16746495,
  16756480,
  16756575,
  16756615,
  16756655,
  16756695,
  16756735,
  16766720,
  16766815,
  16766855,
  16766895,
  16766935,
  16766975,
  16776960,
  16777055,
  16777095,
  16777135,
  16777175,
  16777215,
  526344,
  1184274,
  1842204,
  2500134,
  3158064,
  3815994,
  4473924,
  5131854,
  5789784,
  6447714,
  7105644,
  7763574,
  8421504,
  9079434,
  9737364,
  10395294,
  11053224,
  11711154,
  12369084,
  13027014,
  13684944,
  14342874,
  15000804,
  15658734
];
function hexNumberToRgb(hex) {
  return [
    hex >> 16 & 255,
    hex >> 8 & 255,
    hex & 255
  ];
}
function parseHexString(str) {
  if (!str) return void 0;
  let clean = str.startsWith("#") ? str.slice(1) : str;
  if (clean.length === 3) {
    clean = clean.split("").map((c) => c + c).join("");
  }
  if (clean.length !== 6) return void 0;
  const num = parseInt(clean, 16);
  if (Number.isNaN(num)) return void 0;
  return hexNumberToRgb(num);
}
var IRC99_PALETTE = IRC99_HEX.map(hexNumberToRgb);
var MIRC16_PALETTE = IRC99_PALETTE.slice(0, 16);
var ANSI256_PALETTE = ANSI256_HEX.map(hexNumberToRgb);
var ANSI16_PALETTE = ANSI256_PALETTE.slice(0, 16);
function rgbToString(color, fallback = [0, 0, 0]) {
  const [r, g, b] = color ?? fallback;
  return `rgb(${r} ${g} ${b})`;
}
function rgbToHex(color, fallback = "#000000") {
  if (!color) return fallback;
  const [r, g, b] = color;
  return "#" + [r, g, b].map((x) => Math.max(0, Math.min(255, Math.round(x))).toString(16).padStart(2, "0")).join("");
}
function getIrcColor(index, palette = IRC99_PALETTE) {
  if (index >= 0 && index < palette.length) {
    return palette[index];
  }
  if (palette.length > 0) {
    return palette[index % palette.length];
  }
  return void 0;
}
function getAnsiColor(index, palette = ANSI256_PALETTE) {
  if (index >= 0 && index < palette.length) {
    return palette[index];
  }
  if (palette.length > 0) {
    return palette[index % palette.length];
  }
  return void 0;
}

// ../irc2html/src/parser-irc.js
function parseIrc(text, options = {}) {
  const palette = options.palette ?? IRC99_PALETTE;
  const defaultForeground = options.defaultForeground ?? [255, 255, 255];
  const defaultBackground = options.defaultBackground;
  const tabWidth = Math.max(1, options.tabWidth ?? 8);
  const grid = [];
  let currentRow = [];
  let fg = defaultForeground;
  let bg = defaultBackground;
  let bold = false;
  let italic = false;
  let underline = false;
  let inverted = false;
  let strikethrough = false;
  const resetStyles = () => {
    fg = defaultForeground;
    bg = defaultBackground;
    bold = false;
    italic = false;
    underline = false;
    inverted = false;
    strikethrough = false;
  };
  const createCell = (char) => ({
    character: char,
    foreground: fg ? [...fg] : void 0,
    background: bg ? [...bg] : void 0,
    bold,
    italic,
    underline,
    inverted,
    strikethrough
  });
  const finishRow = () => {
    if (options.trimTrailingSpaces) {
      while (currentRow.length > 0 && currentRow[currentRow.length - 1].character === " ") {
        currentRow.pop();
      }
    }
    grid.push(currentRow);
    currentRow = [];
  };
  if (!text) {
    return { columns: 0, rows: 0, cells: [] };
  }
  const chars = Array.from(text);
  const len = chars.length;
  let i = 0;
  while (i < len) {
    const ch = chars[i];
    if (ch === "\r") {
      if (i + 1 < len && chars[i + 1] === "\n") {
        i++;
      }
      finishRow();
      resetStyles();
      i++;
      continue;
    }
    if (ch === "\n") {
      finishRow();
      resetStyles();
      i++;
      continue;
    }
    if (ch === "	") {
      const spacesToAdd = tabWidth - currentRow.length % tabWidth;
      for (let s = 0; s < spacesToAdd; s++) {
        currentRow.push(createCell(" "));
      }
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      resetStyles();
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      bold = !bold;
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      italic = !italic;
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      underline = !underline;
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      inverted = !inverted;
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      strikethrough = !strikethrough;
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      i++;
      continue;
    }
    if (ch === "" || ch === "") {
      i++;
      let hexFg = "";
      while (hexFg.length < 6 && i < len && /[0-9a-fA-F]/.test(chars[i])) {
        hexFg += chars[i];
        i++;
      }
      if (hexFg.length === 6) {
        fg = parseHexString(hexFg) ?? defaultForeground;
        if (i < len && chars[i] === ",") {
          i++;
          let hexBg = "";
          while (hexBg.length < 6 && i < len && /[0-9a-fA-F]/.test(chars[i])) {
            hexBg += chars[i];
            i++;
          }
          if (hexBg.length === 6) {
            bg = parseHexString(hexBg);
          } else {
            bg = defaultBackground;
          }
        }
      } else {
        fg = defaultForeground;
        bg = defaultBackground;
      }
      continue;
    }
    if (ch === "" || ch === "") {
      i++;
      let fgStr = "";
      while (fgStr.length < 2 && i < len && /[0-9]/.test(chars[i])) {
        fgStr += chars[i];
        i++;
      }
      if (fgStr.length > 0) {
        const fgNum = parseInt(fgStr, 10);
        fg = getIrcColor(fgNum, palette) ?? defaultForeground;
        if (i < len && chars[i] === ",") {
          if (i + 1 < len && /[0-9]/.test(chars[i + 1])) {
            i++;
            let bgStr = "";
            while (bgStr.length < 2 && i < len && /[0-9]/.test(chars[i])) {
              bgStr += chars[i];
              i++;
            }
            if (bgStr.length > 0) {
              const bgNum = parseInt(bgStr, 10);
              bg = getIrcColor(bgNum, palette);
            }
          }
        }
      } else if (i < len && chars[i] === ",") {
        if (i + 1 < len && /[0-9]/.test(chars[i + 1])) {
          i++;
          let bgStr = "";
          while (bgStr.length < 2 && i < len && /[0-9]/.test(chars[i])) {
            bgStr += chars[i];
            i++;
          }
          if (bgStr.length > 0) {
            const bgNum = parseInt(bgStr, 10);
            bg = getIrcColor(bgNum, palette);
          }
        }
      } else {
        fg = defaultForeground;
        bg = defaultBackground;
      }
      continue;
    }
    currentRow.push(createCell(ch));
    i++;
  }
  if (currentRow.length > 0) {
    finishRow();
  }
  if (options.trimTrailingEmptyRows) {
    while (grid.length > 0 && grid[grid.length - 1].length === 0) {
      grid.pop();
    }
  }
  const columns = grid.reduce((max, row) => Math.max(max, row.length), 0);
  const rows = grid.length;
  return {
    columns,
    rows,
    cells: grid
  };
}

// ../irc2html/src/parser-ansi.js
function parseAnsi(text, options = {}) {
  const palette = options.palette ?? ANSI256_PALETTE;
  const defaultForeground = options.defaultForeground ?? [255, 255, 255];
  const defaultBackground = options.defaultBackground;
  const tabWidth = Math.max(1, options.tabWidth ?? 8);
  const grid = [];
  let currentRow = [];
  let fg = defaultForeground;
  let bg = defaultBackground;
  let bold = false;
  let dim = false;
  let italic = false;
  let underline = false;
  let inverted = false;
  let strikethrough = false;
  let hidden = false;
  const resetStyles = () => {
    fg = defaultForeground;
    bg = defaultBackground;
    bold = false;
    dim = false;
    italic = false;
    underline = false;
    inverted = false;
    strikethrough = false;
    hidden = false;
  };
  const createCell = (char) => ({
    character: char,
    foreground: fg ? [...fg] : void 0,
    background: bg ? [...bg] : void 0,
    bold,
    dim,
    italic,
    underline,
    inverted,
    strikethrough,
    hidden
  });
  const finishRow = () => {
    if (options.trimTrailingSpaces) {
      while (currentRow.length > 0 && currentRow[currentRow.length - 1].character === " ") {
        currentRow.pop();
      }
    }
    grid.push(currentRow);
    currentRow = [];
  };
  if (!text) {
    return { columns: 0, rows: 0, cells: [] };
  }
  const chars = Array.from(text);
  const len = chars.length;
  let i = 0;
  while (i < len) {
    const ch = chars[i];
    if (ch === "\x1B" || ch === "\x1B") {
      i++;
      if (i >= len) break;
      if (chars[i] === "[") {
        i++;
        let paramStr = "";
        while (i < len) {
          const c = chars[i];
          const code = c.charCodeAt(0);
          if (code >= 64 && code <= 126) {
            const command = c;
            i++;
            if (command === "m") {
              const params = paramStr.length === 0 ? [0] : paramStr.split(/[:;]/).map((s) => s.trim() === "" ? 0 : parseInt(s, 10)).filter((n) => !Number.isNaN(n));
              if (params.length === 0) {
                resetStyles();
              } else {
                let p = 0;
                while (p < params.length) {
                  const code2 = params[p];
                  if (code2 === 0) {
                    resetStyles();
                  } else if (code2 === 1) {
                    bold = true;
                  } else if (code2 === 2) {
                    dim = true;
                  } else if (code2 === 3) {
                    italic = true;
                  } else if (code2 === 4) {
                    underline = true;
                  } else if (code2 === 7) {
                    inverted = true;
                  } else if (code2 === 8) {
                    hidden = true;
                  } else if (code2 === 9) {
                    strikethrough = true;
                  } else if (code2 === 21) {
                    bold = false;
                  } else if (code2 === 22) {
                    bold = false;
                    dim = false;
                  } else if (code2 === 23) {
                    italic = false;
                  } else if (code2 === 24) {
                    underline = false;
                  } else if (code2 === 27) {
                    inverted = false;
                  } else if (code2 === 28) {
                    hidden = false;
                  } else if (code2 === 29) {
                    strikethrough = false;
                  } else if (code2 >= 30 && code2 <= 37) {
                    fg = getAnsiColor(code2 - 30, palette) ?? defaultForeground;
                  } else if (code2 === 38) {
                    if (p + 1 < params.length) {
                      const type = params[p + 1];
                      if (type === 5 && p + 2 < params.length) {
                        const colorIdx = params[p + 2];
                        fg = getAnsiColor(colorIdx, palette) ?? defaultForeground;
                        p += 2;
                      } else if (type === 2 && p + 4 < params.length) {
                        const r = Math.max(0, Math.min(255, params[p + 2]));
                        const g = Math.max(0, Math.min(255, params[p + 3]));
                        const b = Math.max(0, Math.min(255, params[p + 4]));
                        fg = [r, g, b];
                        p += 4;
                      }
                    }
                  } else if (code2 === 39) {
                    fg = defaultForeground;
                  } else if (code2 >= 40 && code2 <= 47) {
                    bg = getAnsiColor(code2 - 40, palette);
                  } else if (code2 === 48) {
                    if (p + 1 < params.length) {
                      const type = params[p + 1];
                      if (type === 5 && p + 2 < params.length) {
                        const colorIdx = params[p + 2];
                        bg = getAnsiColor(colorIdx, palette);
                        p += 2;
                      } else if (type === 2 && p + 4 < params.length) {
                        const r = Math.max(0, Math.min(255, params[p + 2]));
                        const g = Math.max(0, Math.min(255, params[p + 3]));
                        const b = Math.max(0, Math.min(255, params[p + 4]));
                        bg = [r, g, b];
                        p += 4;
                      }
                    }
                  } else if (code2 === 49) {
                    bg = defaultBackground;
                  } else if (code2 >= 90 && code2 <= 97) {
                    fg = getAnsiColor(code2 - 90 + 8, palette) ?? defaultForeground;
                  } else if (code2 >= 100 && code2 <= 107) {
                    bg = getAnsiColor(code2 - 100 + 8, palette);
                  }
                  p++;
                }
              }
            }
            break;
          } else {
            paramStr += c;
            i++;
          }
        }
        continue;
      }
      if (chars[i] === "]") {
        i++;
        while (i < len) {
          if (chars[i] === "\x07") {
            i++;
            break;
          }
          if (chars[i] === "\x1B" && i + 1 < len && chars[i + 1] === "\\") {
            i += 2;
            break;
          }
          i++;
        }
        continue;
      }
      i++;
      continue;
    }
    if (ch === "\r") {
      if (i + 1 < len && chars[i + 1] === "\n") {
        i++;
      }
      finishRow();
      i++;
      continue;
    }
    if (ch === "\n") {
      finishRow();
      i++;
      continue;
    }
    if (ch === "	") {
      const spacesToAdd = tabWidth - currentRow.length % tabWidth;
      for (let s = 0; s < spacesToAdd; s++) {
        currentRow.push(createCell(" "));
      }
      i++;
      continue;
    }
    currentRow.push(createCell(ch));
    i++;
  }
  if (currentRow.length > 0) {
    finishRow();
  }
  if (options.trimTrailingEmptyRows) {
    while (grid.length > 0 && grid[grid.length - 1].length === 0) {
      grid.pop();
    }
  }
  const columns = grid.reduce((max, row) => Math.max(max, row.length), 0);
  const rows = grid.length;
  return {
    columns,
    rows,
    cells: grid
  };
}

// ../irc2html/src/utils.js
function escapeHtml(str) {
  if (!str) return "";
  return str.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#39;");
}
function displayStyle(cell, defaultFg = [255, 255, 255], defaultBg = [0, 0, 0]) {
  const rawFg = cell.foreground ?? defaultFg;
  const rawBg = cell.background ?? defaultBg;
  let foreground = cell.inverted ? rawBg : rawFg;
  let background = cell.inverted ? rawFg : cell.background ?? defaultBg;
  if (cell.dim) {
    foreground = [
      Math.round(foreground[0] * 0.7),
      Math.round(foreground[1] * 0.7),
      Math.round(foreground[2] * 0.7)
    ];
  }
  return {
    foreground,
    background,
    bold: Boolean(cell.bold),
    dim: Boolean(cell.dim),
    italic: Boolean(cell.italic),
    underline: Boolean(cell.underline),
    strikethrough: Boolean(cell.strikethrough),
    hidden: Boolean(cell.hidden)
  };
}
function sameStyle(left, right) {
  if (!left || !right) return false;
  return left.bold === right.bold && left.dim === right.dim && left.italic === right.italic && left.underline === right.underline && left.strikethrough === right.strikethrough && left.hidden === right.hidden && left.foreground[0] === right.foreground[0] && left.foreground[1] === right.foreground[1] && left.foreground[2] === right.foreground[2] && left.background[0] === right.background[0] && left.background[1] === right.background[1] && left.background[2] === right.background[2];
}
function detectFormat(text) {
  if (!text || typeof text !== "string") return "plain";
  if (/\x1b\[|\u001b\[|\x1b\]|\u001b\]/.test(text)) {
    return "ansi";
  }
  if (/[\x02\x03\x04\x0F\x11\x16\x1D\x1E\x1F]/.test(text)) {
    return "irc";
  }
  return "plain";
}

// ../irc2html/src/parser.js
function parse(input, options = {}) {
  if (!input) {
    return { columns: 0, rows: 0, cells: [] };
  }
  if (Array.isArray(input)) {
    const columns = input.reduce((max, row) => Math.max(max, Array.isArray(row) ? row.length : 0), 0);
    return {
      columns,
      rows: input.length,
      cells: input
    };
  }
  if (typeof input === "object") {
    const rawCells = input.cells ?? input.grid;
    if (Array.isArray(rawCells)) {
      const columns = input.columns ?? rawCells.reduce((max, row) => Math.max(max, Array.isArray(row) ? row.length : 0), 0);
      const rows = input.rows ?? rawCells.length;
      return {
        columns,
        rows,
        cells: rawCells
      };
    }
  }
  if (typeof input !== "string") {
    return { columns: 0, rows: 0, cells: [] };
  }
  const format = options.format && options.format !== "auto" ? options.format : detectFormat(input);
  if (format === "ansi") {
    return parseAnsi(input, options);
  }
  return parseIrc(input, options);
}

// ../irc2html/src/styles.js
var DEFAULT_CSS = `
.output-canvas {
  background: #000;
  display: block;
  flex: none;
  image-rendering: pixelated;
}

.output-canvas[hidden],
.output-text[hidden] {
  display: none !important;
}

.output-text,
.irc2html-output {
  position: relative;
  background: #000;
  color: #fff;
  contain: layout paint style;
  display: inline-block;
  flex: none;
  font-family: "Iosevka Fixed", monospace;
  font-kerning: none;
  font-size: 16px;
  font-synthesis: style weight;
  font-variant-ligatures: none;
  font-feature-settings: "liga" 0, "calt" 0;
  text-rendering: optimizeSpeed;
  user-select: text;
  box-sizing: border-box;
}

.output-text-row,
.irc2html-row {
  display: flex;
  height: var(--output-line-height, var(--irc2html-line-height));
  line-height: var(--output-line-height, var(--irc2html-line-height));
  overflow: hidden;
  white-space: pre;
}

.output-text-run,
.irc2html-run {
  display: flex;
  flex: none;
  height: var(--output-line-height, var(--irc2html-line-height));
  line-height: var(--output-line-height, var(--irc2html-line-height));
  /* Preserve the original native-text seam treatment inside clipped cells. */
  text-shadow:
    -.1px 0 currentColor,
    .1px 0 currentColor,
    0 -.1px currentColor,
    0 .1px currentColor;
  white-space: pre;
}

.output-text-cell,
.irc2html-cell {
  position: relative;
  box-sizing: border-box;
  display: block;
  flex: 0 0 var(--output-cell-advance, var(--irc2html-cell-advance));
  height: var(--output-line-height, var(--irc2html-line-height));
  line-height: var(--output-line-height, var(--irc2html-line-height));
  max-width: var(--output-cell-advance, var(--irc2html-cell-advance));
  min-width: 0;
  overflow: hidden;
  white-space: pre;
  width: var(--output-cell-advance, var(--irc2html-cell-advance));
}

.irc2html-glyph {
  position: absolute;
  display: block;
  left: 0;
  top: 0;
  width: var(--irc2html-font-advance);
  height: var(--irc2html-font-height);
  line-height: var(--irc2html-font-height);
  letter-spacing: 0;
  transform-origin: 0 0;
  transform: scale(calc(var(--irc2html-scale-x, 1) * var(--irc2html-cell-scale-x, 1)), calc(var(--irc2html-scale-y, 1) * var(--irc2html-cell-scale-y, 1))) translateY(var(--irc2html-baseline-offset, 0px));
}

`.trim();
function injectDefaultStyles(doc = typeof document !== "undefined" ? document : void 0) {
  if (!doc || !doc.head) return void 0;
  let styleEl = doc.getElementById("irc2html-styles");
  if (!styleEl) {
    styleEl = doc.createElement("style");
    styleEl.id = "irc2html-styles";
    doc.head.appendChild(styleEl);
  }
  if (styleEl.textContent !== DEFAULT_CSS) styleEl.textContent = DEFAULT_CSS;
  return styleEl;
}

// ../irc2html/src/metrics.js
var context;
var cache = /* @__PURE__ */ new Map();
function fontFamilyCss(family = "Iosevka Fixed, monospace") {
  const parts = family.split(",").map((part) => {
    const name = part.trim().replace(/^(['"])(.*)\1$/, "$2");
    return /^(monospace|serif|sans-serif|system-ui|cursive|fantasy)$/i.test(name) ? name : JSON.stringify(name);
  });
  if (!parts.some((part) => /^(monospace|serif|sans-serif|system-ui|cursive|fantasy)$/i.test(part))) parts.push("monospace");
  return parts.join(", ");
}
function measurementContext() {
  if (context || typeof document === "undefined") return context;
  context = document.createElement("canvas").getContext("2d");
  document.fonts?.addEventListener("loadingdone", () => cache.clear());
  return context;
}
function fontMetrics(fontSize, fontFamily) {
  const family = fontFamilyCss(fontFamily);
  const key = `${fontSize}:${family}`;
  if (cache.has(key)) return cache.get(key);
  const ctx = measurementContext();
  let advance = fontSize * 0.5;
  let ascent = fontSize;
  let descent = 0;
  if (ctx) {
    ctx.font = `400 ${fontSize}px ${family}`;
    const m = ctx.measureText("M");
    advance = m.width || advance;
    ctx.font = `400 ${fontSize * 64}px ${family}`;
    const reference = ctx.measureText("M");
    ascent = reference.fontBoundingBoxAscent === void 0 ? ascent : reference.fontBoundingBoxAscent / 64;
    descent = reference.fontBoundingBoxDescent === void 0 ? descent : reference.fontBoundingBoxDescent / 64;
  }
  const height = Math.max(1, ascent + descent);
  let baseline = ascent;
  if (typeof document !== "undefined" && document.body) {
    const probe = document.createElement("span");
    probe.style.cssText = `position:fixed;left:-10000px;top:0;visibility:hidden;display:inline-block;font:400 ${fontSize}px ${family};line-height:${height}px;letter-spacing:0;white-space:pre;`;
    probe.textContent = "M";
    const marker = document.createElement("span");
    marker.style.cssText = "display:inline-block;vertical-align:baseline;width:0;height:0;";
    probe.append(marker);
    document.body.append(probe);
    baseline = marker.getBoundingClientRect().top - probe.getBoundingClientRect().top;
    probe.remove();
  }
  const metrics = { family, advance, height, ascent, baselineOffset: ascent - baseline };
  if (cache.size >= 128) cache.clear();
  if (!ctx || document.fonts?.check(`400 ${fontSize}px ${family}`)) cache.set(key, metrics);
  return metrics;
}
function calculateMetrics(options = {}, columns = 0, rows = 0) {
  const requested = parseFloat(options.fontSize ?? 16);
  const fontSize = requested > 0 && Number.isFinite(requested) ? requested : 16;
  const fontFamily = options.fontFamily || "Iosevka Fixed, monospace";
  const cellAdvance = options.cellAdvance ?? fontSize * (options.aspectRatio ?? 0.5);
  const lineHeight = options.lineHeight ?? fontSize;
  return {
    fontSize,
    fontFamily,
    cellAdvance,
    lineHeight,
    letterSpacing: "0px",
    width: columns > 0 ? columns * cellAdvance : void 0,
    height: rows > 0 ? rows * lineHeight : void 0
  };
}
function metricStyles(metrics) {
  const font = fontMetrics(metrics.fontSize, metrics.fontFamily);
  return {
    "font-size": `${metrics.fontSize}px`,
    "font-family": font.family,
    "line-height": `${metrics.lineHeight}px`,
    "letter-spacing": "0px",
    "--irc2html-font-advance": `${font.advance}px`,
    "--irc2html-font-height": `${font.height}px`,
    "--irc2html-baseline-offset": `${font.baselineOffset}px`,
    "--irc2html-scale-x": metrics.cellAdvance / font.advance,
    "--irc2html-scale-y": metrics.lineHeight / font.height,
    "--output-line-height": `${metrics.lineHeight}px`,
    "--output-cell-advance": `${metrics.cellAdvance}px`,
    "--irc2html-line-height": `${metrics.lineHeight}px`,
    "--irc2html-cell-advance": `${metrics.cellAdvance}px`,
    ...metrics.width !== void 0 ? { width: `${pixelBoundary(metrics.width)}px` } : {},
    ...metrics.height !== void 0 ? { height: `${pixelBoundary(metrics.height)}px` } : {}
  };
}
function pixelBoundary(value) {
  const ratio = Math.max(1, globalThis.devicePixelRatio || 1);
  return Math.round(value * ratio) / ratio;
}
function gridLayout(metrics, columns, rows) {
  const width = pixelBoundary(columns * metrics.cellAdvance);
  const height = pixelBoundary(rows * metrics.lineHeight);
  return {
    row: (y) => {
      const top = pixelBoundary(y * metrics.lineHeight);
      const bottom = pixelBoundary((y + 1) * metrics.lineHeight);
      return {
        position: "absolute",
        display: "block",
        left: "0px",
        right: "0px",
        top: `${top}px`,
        bottom: `${height - bottom}px`,
        height: "auto",
        "--irc2html-cell-scale-y": (bottom - top) / metrics.lineHeight
      };
    },
    cell: (x) => {
      const left = pixelBoundary(x * metrics.cellAdvance);
      const right = pixelBoundary((x + 1) * metrics.cellAdvance);
      return {
        position: "absolute",
        left: `${left}px`,
        right: `${width - right}px`,
        width: "auto",
        "max-width": "none",
        height: "100%",
        "background-color": "inherit",
        "--irc2html-cell-scale-x": (right - left) / metrics.cellAdvance
      };
    }
  };
}

// ../irc2html/src/dom-renderer.js
function applyMetricsToElement(element, metrics, columns = 0, rows = 0) {
  if (!element?.style) return;
  const sized = {
    ...metrics,
    width: columns > 0 ? columns * metrics.cellAdvance : void 0,
    height: rows > 0 ? rows * metrics.lineHeight : void 0
  };
  for (const [name, value] of Object.entries(metricStyles(sized))) {
    element.style.setProperty(name, String(value));
  }
  const layout = gridLayout(metrics, columns, rows);
  [...element.children].forEach((row, y) => {
    for (const [name, value] of Object.entries(layout.row(y))) row.style.setProperty(name, String(value));
    let x = 0;
    for (const run of row.children) {
      run.style.display = "contents";
      for (const cell of run.children) {
        for (const [name, value] of Object.entries(layout.cell(x++))) cell.style.setProperty(name, String(value));
      }
    }
  });
}
function buildDOMRows(cellRows, options = {}) {
  if (typeof document === "undefined") {
    throw new Error("buildDOMRows requires a DOM environment with document support.");
  }
  const defaultForeground = options.defaultForeground ?? [255, 255, 255];
  const defaultBackground = options.defaultBackground ?? [0, 0, 0];
  const rowClassName = options.rowClassName ?? "irc2html-row";
  const runClassName = options.runClassName ?? "irc2html-run";
  const cellClassName = options.cellClassName ?? "irc2html-cell";
  const fragment = document.createDocumentFragment();
  for (const cells of cellRows) {
    const rowEl = document.createElement("div");
    rowEl.className = rowClassName;
    let currentRun = null;
    let currentStyle = null;
    for (const cell of cells) {
      const style = displayStyle(cell, defaultForeground, defaultBackground);
      if (!currentRun || !sameStyle(style, currentStyle)) {
        currentRun = document.createElement("span");
        currentRun.className = runClassName;
        currentRun.style.color = rgbToString(style.foreground, defaultForeground);
        currentRun.style.backgroundColor = rgbToString(style.background, defaultBackground);
        if (style.bold) currentRun.style.fontWeight = "700";
        if (style.italic) currentRun.style.fontStyle = "italic";
        if (style.underline) currentRun.style.textDecoration = "underline";
        if (style.strikethrough) {
          currentRun.style.textDecoration = (currentRun.style.textDecoration ? currentRun.style.textDecoration + " " : "") + "line-through";
        }
        if (style.hidden) currentRun.style.visibility = "hidden";
        rowEl.append(currentRun);
        currentStyle = style;
      }
      const cellEl = document.createElement("span");
      cellEl.className = cellClassName;
      const glyphEl = document.createElement("span");
      glyphEl.className = "irc2html-glyph";
      glyphEl.textContent = cell.character ?? "";
      cellEl.append(glyphEl);
      currentRun.append(cellEl);
    }
    fragment.append(rowEl);
  }
  return fragment;
}

// ../irc2html/src/output-preview.js
var _stage, _canvas, _text, _placeholder, _mode, _fontSize, _fontFamily, _options, _result, _metrics, _bitmapSource, _OutputPreview_instances, updateMetrics_fn, resultMetrics_fn, drawBitmap_fn, updateVisibility_fn;
var OutputPreview = class {
  constructor(stageOrTarget, canvasOrOptions, text, placeholder) {
    __privateAdd(this, _OutputPreview_instances);
    __privateAdd(this, _stage);
    __privateAdd(this, _canvas);
    __privateAdd(this, _text);
    __privateAdd(this, _placeholder);
    __privateAdd(this, _mode, "text");
    __privateAdd(this, _fontSize, 16);
    __privateAdd(this, _fontFamily, "Iosevka Fixed, monospace");
    __privateAdd(this, _options, {});
    __privateAdd(this, _result);
    __privateAdd(this, _metrics);
    __privateAdd(this, _bitmapSource);
    if (!stageOrTarget) throw new Error("OutputPreview requires a target DOM element.");
    injectDefaultStyles();
    __privateSet(this, _stage, stageOrTarget);
    if (canvasOrOptions instanceof HTMLElement && text instanceof HTMLElement) {
      __privateSet(this, _canvas, canvasOrOptions);
      __privateSet(this, _text, text);
      __privateSet(this, _placeholder, placeholder);
    } else {
      __privateSet(this, _options, canvasOrOptions || {});
      __privateSet(this, _fontSize, __privateGet(this, _options).fontSize || 16);
      __privateSet(this, _fontFamily, __privateGet(this, _options).fontFamily || __privateGet(this, _fontFamily));
      __privateSet(this, _mode, __privateGet(this, _options).mode === "bitmap" ? "bitmap" : "text");
      __privateSet(this, _canvas, document.createElement("canvas"));
      __privateGet(this, _canvas).className = "output-canvas";
      __privateGet(this, _canvas).setAttribute("role", "img");
      __privateGet(this, _canvas).setAttribute("aria-label", "Rendered terminal art as bitmap canvas");
      __privateSet(this, _text, document.createElement("div"));
      __privateGet(this, _text).className = "output-text";
      __privateGet(this, _text).setAttribute("role", "img");
      __privateGet(this, _text).setAttribute("aria-label", "Rendered terminal art as native browser text");
      __privateGet(this, _stage).replaceChildren(__privateGet(this, _canvas), __privateGet(this, _text));
    }
    __privateMethod(this, _OutputPreview_instances, updateVisibility_fn).call(this);
    document.fonts?.ready.then(() => {
      if (__privateGet(this, _result)) __privateMethod(this, _OutputPreview_instances, updateMetrics_fn).call(this);
    });
  }
  setMode(mode) {
    __privateSet(this, _mode, mode === "bitmap" ? "bitmap" : "text");
    if (__privateGet(this, _mode) === "bitmap" && __privateGet(this, _result)) __privateMethod(this, _OutputPreview_instances, drawBitmap_fn).call(this);
    __privateMethod(this, _OutputPreview_instances, updateVisibility_fn).call(this);
  }
  getMode() {
    return __privateGet(this, _mode);
  }
  setFontFamily(family) {
    __privateSet(this, _fontFamily, family || "Iosevka Fixed, monospace");
    if (__privateGet(this, _result)) __privateMethod(this, _OutputPreview_instances, updateMetrics_fn).call(this);
    const selected = __privateGet(this, _fontFamily);
    document.fonts?.load(`400 ${__privateGet(this, _fontSize)}px ${fontFamilyCss(selected)}`).then(() => {
      if (__privateGet(this, _fontFamily) === selected && __privateGet(this, _result)) __privateMethod(this, _OutputPreview_instances, updateMetrics_fn).call(this);
    }).catch(() => {
    });
  }
  setOutputFontSize(value) {
    const size = Number(value);
    if (!(size > 0) || !Number.isFinite(size)) return;
    __privateSet(this, _fontSize, size);
    if (__privateGet(this, _result)) __privateMethod(this, _OutputPreview_instances, updateMetrics_fn).call(this);
  }
  draw(resultOrArt, options = {}) {
    const raw = typeof resultOrArt === "string";
    const opts = { ...__privateGet(this, _options), ...options };
    if (options.fontSize > 0) __privateSet(this, _fontSize, Number(options.fontSize));
    if (options.fontFamily) __privateSet(this, _fontFamily, options.fontFamily);
    if (raw) {
      const grid = parse(resultOrArt, opts);
      const metrics = calculateMetrics({ ...opts, fontSize: opts.fontSize ?? __privateGet(this, _fontSize) });
      __privateSet(this, _result, { ...grid, ...metrics, renderFontSize: metrics.fontSize });
    } else {
      __privateSet(this, _result, resultOrArt);
    }
    __privateSet(this, _bitmapSource, void 0);
    if (__privateGet(this, _result)?.cells) {
      __privateGet(this, _text).replaceChildren(buildDOMRows(__privateGet(this, _result).cells, {
        ...opts,
        rowClassName: "output-text-row",
        runClassName: "output-text-run",
        cellClassName: "output-text-cell"
      }));
    } else {
      __privateGet(this, _text).replaceChildren();
      if (__privateGet(this, _result)?.previewRgba?.length) __privateSet(this, _bitmapSource, __privateGet(this, _result));
    }
    __privateMethod(this, _OutputPreview_instances, updateMetrics_fn).call(this);
  }
  render(resultOrArt, options) {
    this.draw(resultOrArt, options);
  }
  hasText() {
    return Boolean(__privateGet(this, _result)?.cells?.length);
  }
  toPngBlob(result = __privateGet(this, _result)) {
    if (!result) return Promise.reject(new Error("Render an image before saving PNG"));
    const canvas = document.createElement("canvas");
    __privateMethod(this, _OutputPreview_instances, drawBitmap_fn).call(this, result, __privateMethod(this, _OutputPreview_instances, resultMetrics_fn).call(this, result), canvas, result === __privateGet(this, _result) ? __privateGet(this, _bitmapSource) : null);
    return new Promise((resolve, reject) => canvas.toBlob(
      (blob) => blob ? resolve(blob) : reject(new Error("Could not create PNG")),
      "image/png"
    ));
  }
  clear(message = "Rendered output will appear here.") {
    __privateSet(this, _result, void 0);
    __privateSet(this, _metrics, void 0);
    __privateSet(this, _bitmapSource, void 0);
    __privateGet(this, _text).replaceChildren();
    __privateGet(this, _canvas).width = 0;
    __privateGet(this, _canvas).height = 0;
    __privateMethod(this, _OutputPreview_instances, updateVisibility_fn).call(this);
    if (__privateGet(this, _placeholder)) __privateGet(this, _placeholder).textContent = message;
  }
  clientPointToCell(clientX, clientY, columns, rows) {
    const bounds = __privateGet(this, _stage).getBoundingClientRect();
    const cols = columns ?? __privateGet(this, _result)?.columns ?? 0;
    const count = rows ?? __privateGet(this, _result)?.rows ?? 0;
    if (bounds.width <= 0 || bounds.height <= 0 || cols <= 0 || count <= 0 || clientX < bounds.left || clientX >= bounds.right || clientY < bounds.top || clientY >= bounds.bottom) return void 0;
    return {
      x: Math.min(cols - 1, Math.floor((clientX - bounds.left) * cols / bounds.width)),
      y: Math.min(count - 1, Math.floor((clientY - bounds.top) * count / bounds.height))
    };
  }
};
_stage = new WeakMap();
_canvas = new WeakMap();
_text = new WeakMap();
_placeholder = new WeakMap();
_mode = new WeakMap();
_fontSize = new WeakMap();
_fontFamily = new WeakMap();
_options = new WeakMap();
_result = new WeakMap();
_metrics = new WeakMap();
_bitmapSource = new WeakMap();
_OutputPreview_instances = new WeakSet();
updateMetrics_fn = function() {
  const result = __privateGet(this, _result);
  if (!result) return;
  __privateSet(this, _metrics, __privateMethod(this, _OutputPreview_instances, resultMetrics_fn).call(this, result));
  applyMetricsToElement(__privateGet(this, _text), __privateGet(this, _metrics), result.columns, result.rows);
  if (__privateGet(this, _mode) === "bitmap") __privateMethod(this, _OutputPreview_instances, drawBitmap_fn).call(this);
  __privateMethod(this, _OutputPreview_instances, updateVisibility_fn).call(this);
};
resultMetrics_fn = function(result) {
  const scale = __privateGet(this, _fontSize) / (result.renderFontSize || result.fontSize || __privateGet(this, _fontSize));
  return calculateMetrics({
    fontSize: __privateGet(this, _fontSize),
    fontFamily: __privateGet(this, _fontFamily),
    cellAdvance: result.cellAdvance === void 0 ? void 0 : result.cellAdvance * scale,
    lineHeight: result.lineHeight === void 0 ? void 0 : result.lineHeight * scale
  }, result.columns, result.rows);
};
drawBitmap_fn = function(result = __privateGet(this, _result), metrics = __privateGet(this, _metrics), canvas = __privateGet(this, _canvas), bitmapSource = __privateGet(this, _bitmapSource)) {
  if (!result || !metrics) return;
  const ratio = Math.max(1, globalThis.devicePixelRatio || 1);
  const width = Math.max(1, Math.round(metrics.width * ratio));
  const height = Math.max(1, Math.round(metrics.height * ratio));
  canvas.width = width;
  canvas.height = height;
  canvas.style.width = `${metrics.width}px`;
  canvas.style.height = `${metrics.height}px`;
  canvas.style.imageRendering = "auto";
  const ctx = canvas.getContext("2d", { alpha: false });
  if (bitmapSource) {
    const source = bitmapSource;
    const image = document.createElement("canvas");
    image.width = source.previewWidth;
    image.height = source.previewHeight;
    image.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(
      source.previewRgba.buffer,
      source.previewRgba.byteOffset,
      source.previewRgba.byteLength
    ), source.previewWidth, source.previewHeight), 0, 0);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(image, 0, 0, width, height);
    return;
  }
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, width, height);
  const family = fontFamilyCss(__privateGet(this, _fontFamily));
  const font = fontMetrics(metrics.fontSize, __privateGet(this, _fontFamily));
  ctx.textBaseline = "alphabetic";
  ctx.fontKerning = "none";
  for (let y = 0; y < result.cells.length; y++) {
    const top = Math.round(y * metrics.lineHeight * ratio);
    const bottom = Math.round((y + 1) * metrics.lineHeight * ratio);
    for (let x = 0; x < result.cells[y].length; x++) {
      const cell = result.cells[y][x];
      const style = displayStyle(cell, [255, 255, 255], [0, 0, 0]);
      const left = Math.round(x * metrics.cellAdvance * ratio);
      const right = Math.round((x + 1) * metrics.cellAdvance * ratio);
      ctx.fillStyle = rgbToString(style.background);
      ctx.fillRect(left, top, right - left, bottom - top);
      if (style.hidden || !cell.character || cell.character === " ") continue;
      ctx.save();
      ctx.beginPath();
      ctx.rect(left, top, right - left, bottom - top);
      ctx.clip();
      ctx.setTransform((right - left) / font.advance, 0, 0, (bottom - top) / font.height, left, top);
      ctx.fillStyle = rgbToString(style.foreground);
      ctx.font = `${style.italic ? "italic " : ""}${style.bold ? "700" : "400"} ${metrics.fontSize}px ${family}`;
      ctx.fillText(cell.character, 0, font.ascent);
      ctx.restore();
    }
  }
};
updateVisibility_fn = function() {
  const available = this.hasText() || Boolean(__privateGet(this, _bitmapSource));
  __privateGet(this, _text).hidden = __privateGet(this, _mode) !== "text" || !this.hasText();
  __privateGet(this, _canvas).hidden = __privateGet(this, _mode) !== "bitmap" || !available;
  __privateGet(this, _stage).style.width = `${available ? __privateGet(this, _metrics)?.width || 0 : 0}px`;
  __privateGet(this, _stage).style.height = `${available ? __privateGet(this, _metrics)?.height || 0 : 0}px`;
  if (__privateGet(this, _placeholder)) __privateGet(this, _placeholder).hidden = available;
};
function render(art, targetElement, options = {}) {
  const preview = new OutputPreview(targetElement, options);
  preview.draw(art, options);
  return preview;
}
function renderTo(art, targetElement, options = {}) {
  return render(art, targetElement, options);
}

// ../irc2html/src/html-renderer.js
function toHtml(input, options = {}) {
  const parsed = parse(input, options);
  const defaultForeground = options.defaultForeground ?? [255, 255, 255];
  const defaultBackground = options.defaultBackground ?? [0, 0, 0];
  const className = options.className ?? "output-text";
  const rowClassName = options.rowClassName ?? "output-text-row";
  const runClassName = options.runClassName ?? "output-text-run";
  const cellClassName = options.cellClassName ?? "output-text-cell";
  const metrics = calculateMetrics(options, parsed.columns, parsed.rows);
  const containerStyles = Object.entries(metricStyles(metrics)).map(([name, value]) => `${name}: ${value};`);
  const rowsHtml = [];
  const layout = gridLayout(metrics, parsed.columns, parsed.rows);
  const styleAttr = (styles) => options.inlineStyles !== false ? ` style="${escapeHtml(Object.entries(styles).map(([name, value]) => `${name}: ${value};`).join(" "))}"` : "";
  for (const [y, cells] of parsed.cells.entries()) {
    let currentRunCells = [];
    let currentStyle = null;
    let runsHtml = [];
    const flushRun = () => {
      if (currentStyle && currentRunCells.length > 0) {
        const styleDeclarations = [
          "display: contents;",
          `color: ${rgbToString(currentStyle.foreground, defaultForeground)};`,
          `background-color: ${rgbToString(currentStyle.background, defaultBackground)};`
        ];
        if (currentStyle.bold) styleDeclarations.push("font-weight: 700;");
        if (currentStyle.italic) styleDeclarations.push("font-style: italic;");
        if (currentStyle.underline) styleDeclarations.push("text-decoration: underline;");
        const styleAttr2 = options.inlineStyles !== false ? ` style="${styleDeclarations.join(" ")}"` : "";
        runsHtml.push(
          `<span class="${escapeHtml(runClassName)}"${styleAttr2}>${currentRunCells.join("")}</span>`
        );
        currentRunCells = [];
      }
    };
    for (const [x, cell] of cells.entries()) {
      const style = displayStyle(cell, defaultForeground, defaultBackground);
      if (!currentStyle || !sameStyle(style, currentStyle)) {
        flushRun();
        currentStyle = style;
      }
      currentRunCells.push(
        `<span class="${escapeHtml(cellClassName)}"${styleAttr(layout.cell(x))}><span class="irc2html-glyph">${escapeHtml(cell.character ?? "")}</span></span>`
      );
    }
    flushRun();
    rowsHtml.push(
      `  <div class="${escapeHtml(rowClassName)}"${styleAttr(layout.row(y))}>${runsHtml.join("")}</div>`
    );
  }
  const containerStyleAttr = options.inlineStyles !== false ? ` style="${escapeHtml(containerStyles.join(" "))}"` : "";
  let output = `<div class="${escapeHtml(className)}"${containerStyleAttr} role="img" aria-label="${escapeHtml(options.ariaLabel ?? "Rendered terminal art as native browser text")}">
${rowsHtml.join("\n")}
</div>`;
  if (options.includeCss) {
    output = `<style>
${DEFAULT_CSS}
</style>
` + output;
  }
  return output;
}
function toHtmlDocument(input, options = {}) {
  const title = options.title ?? "Terminal Art";
  const bodyContent = toHtml(input, { ...options, includeCss: false });
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${escapeHtml(title)}</title>
  <style>
    body {
      margin: 0;
      padding: 24px;
      background: #000;
      color: #fff;
      display: flex;
      justify-content: center;
      align-items: center;
      min-height: 100vh;
      box-sizing: border-box;
    }
${DEFAULT_CSS}
  </style>
</head>
<body>
${bodyContent}
</body>
</html>
`;
}
export {
  ANSI16_PALETTE,
  ANSI256_HEX,
  ANSI256_PALETTE,
  DEFAULT_CSS,
  IRC99_HEX,
  IRC99_PALETTE,
  MIRC16_PALETTE,
  OutputPreview,
  detectFormat,
  displayStyle,
  escapeHtml,
  getAnsiColor,
  getIrcColor,
  hexNumberToRgb,
  injectDefaultStyles,
  parse,
  parseAnsi,
  parseHexString,
  parseIrc,
  render,
  renderTo,
  rgbToHex,
  rgbToString,
  sameStyle,
  toHtml,
  toHtmlDocument
};
