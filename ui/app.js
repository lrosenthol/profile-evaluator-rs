const sourceJsonField = document.getElementById("source-json");
const profileYamlField = document.getElementById("profile-yaml");
const evaluateButton = document.getElementById("evaluate-button");
const jsonTree = document.getElementById("json-tree");
const visualView = document.getElementById("visual-view");
const rawJsonView = document.getElementById("raw-json-view");
const inputsRow = document.querySelector(".inputs-row");
const outputPanel = document.querySelector(".output-panel");
const verticalResizer = document.getElementById("vertical-resizer");
const status = document.getElementById("status");
const tabVisual = document.getElementById("tab-visual");
const tabJson = document.getElementById("tab-json");
const tabRawJson = document.getElementById("tab-raw-json");

const selectJsonButton = document.getElementById("select-json");
const selectYamlButton = document.getElementById("select-yaml");
const selectedJsonPath = document.getElementById("selected-json-path");
const selectedYamlPath = document.getElementById("selected-yaml-path");
const jsonFileInput = document.getElementById("json-file-input");
const yamlFileInput = document.getElementById("yaml-file-input");

let yamlPath = null;

function setStatus(message, isError = false) {
  status.textContent = message;
  status.classList.toggle("error", isError);
}

async function createRuntime() {
  if (window.__TAURI__?.core?.invoke) {
    return {
      kind: "tauri",
      selectFile(kind) {
        return window.__TAURI__.core.invoke("select_and_load_file", { kind });
      },
      evaluateProfile({ sourceJson, profileYaml, profilePath }) {
        return window.__TAURI__.core.invoke("evaluate_profile", {
          sourceJson,
          profileYaml,
          profilePath,
        });
      },
    };
  }

  setStatus("Loading WASM runtime...");
  evaluateButton.disabled = true;

  try {
    const wasmModule = await import("./pkg/profile_evaluator_rs.js");
    const wasmUrl = new URL("./pkg/profile_evaluator_rs_bg.wasm", import.meta.url);
    await wasmModule.default(wasmUrl);
    setStatus("WASM runtime ready");

    return {
      kind: "browser",
      selectFile(kind) {
        return browserSelectFile(kind);
      },
      evaluateProfile({ sourceJson, profileYaml }) {
        const jsonString = wasmModule.evaluate_profile_wasm(profileYaml, sourceJson);
        return JSON.parse(jsonString);
      },
    };
  } catch (error) {
    const message =
      "Failed to load the WASM bundle. Run `./scripts/build-wasm.sh` first and serve `ui/` over HTTP.";
    setStatus(message, true);
    throw new Error(`${message} ${String(error)}`);
  } finally {
    evaluateButton.disabled = false;
  }
}

function browserSelectFile(kind) {
  const input = kind === "json" ? jsonFileInput : yamlFileInput;
  if (!input) {
    return Promise.reject(new Error(`Missing file input for ${kind}`));
  }

  return new Promise((resolve, reject) => {
    const handleChange = async () => {
      input.removeEventListener("change", handleChange);
      const [file] = Array.from(input.files || []);
      input.value = "";

      if (!file) {
        resolve(null);
        return;
      }

      try {
        resolve({
          path: file.name,
          contents: await file.text(),
        });
      } catch (error) {
        reject(error);
      }
    };

    input.addEventListener("change", handleChange, { once: true });
    input.click();
  });
}

function createKeyLabel(key) {
  const keySpan = document.createElement("span");
  keySpan.className = "key";
  keySpan.textContent = `"${key}"`;
  return keySpan;
}

function createPrimitiveNode(value) {
  const span = document.createElement("span");

  if (typeof value === "string") {
    span.className = "string";
    span.textContent = JSON.stringify(value);
    return span;
  }

  if (typeof value === "number") {
    span.className = "number";
    span.textContent = String(value);
    return span;
  }

  if (typeof value === "boolean") {
    span.className = "boolean";
    span.textContent = value ? "true" : "false";
    return span;
  }

  span.className = "null";
  span.textContent = "null";
  return span;
}

function renderTree(value, container, depth = 0) {
  if (value === null || typeof value !== "object") {
    container.appendChild(createPrimitiveNode(value));
    return;
  }

  const isArray = Array.isArray(value);
  const entries = isArray ? value.map((v, i) => [i, v]) : Object.entries(value);

  const details = document.createElement("details");
  details.open = depth < 1;

  const summary = document.createElement("summary");
  summary.textContent = isArray ? `[${entries.length}]` : `{${entries.length}}`;
  details.appendChild(summary);

  const list = document.createElement("ul");

  for (const [key, childValue] of entries) {
    const item = document.createElement("li");

    if (isArray) {
      const index = document.createElement("span");
      index.className = "key";
      index.textContent = `${key}: `;
      item.appendChild(index);
    } else {
      item.appendChild(createKeyLabel(key));
      item.appendChild(document.createTextNode(": "));
    }

    if (childValue !== null && typeof childValue === "object") {
      renderTree(childValue, item, depth + 1);
    } else {
      item.appendChild(createPrimitiveNode(childValue));
    }

    list.appendChild(item);
  }

  details.appendChild(list);
  container.appendChild(details);
}

function renderJsonResult(value) {
  jsonTree.innerHTML = "";
  renderTree(value, jsonTree);
}

function escapeHtml(input) {
  return input
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function syntaxHighlightRawJson(value) {
  const raw = JSON.stringify(value, null, 2);
  const escaped = escapeHtml(raw);
  return escaped.replace(
    /("(?:\\u[\da-fA-F]{4}|\\[^u]|[^\\"])*")(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+\-]?\d+)?|[{}[\],:]/g,
    (match, quoted, keyColon, keyword) => {
      if (/^[{}[\],:]$/.test(match)) {
        return `<span class="json-punc">${match}</span>`;
      }
      if (quoted) {
        if (keyColon) {
          return `<span class="json-key">${quoted}</span>${keyColon}`;
        }
        return `<span class="json-string">${quoted}</span>`;
      }
      if (keyword) {
        if (keyword === "null") {
          return '<span class="json-null">null</span>';
        }
        return `<span class="json-boolean">${keyword}</span>`;
      }
      return `<span class="json-number">${match}</span>`;
    },
  );
}

function renderRawJsonResult(value) {
  rawJsonView.innerHTML = syntaxHighlightRawJson(value);
}

function formatValueText(value) {
  if (typeof value === "string") {
    return value;
  }
  if (
    typeof value === "number" ||
    typeof value === "boolean" ||
    value === null
  ) {
    return String(value);
  }
  return JSON.stringify(value);
}

function createConformanceSummary(label, value) {
  const summary = document.createElement("div");
  summary.className = "summary-card";

  const heading = document.createElement("h3");
  heading.className = "summary-title";
  heading.textContent = label;
  summary.appendChild(heading);

  const badge = document.createElement("div");
  badge.className = "conformance-badge";
  if (value === true) {
    badge.classList.add("pass");
    badge.textContent = "✅ Conformant";
  } else if (value === false) {
    badge.classList.add("fail");
    badge.textContent = "❌ Non-conformant";
  } else {
    badge.classList.add("fail");
    badge.textContent = `ℹ️ ${formatValueText(value)}`;
  }

  summary.appendChild(badge);
  return summary;
}

function findProfileQualitySummary(result) {
  const sections = Array.isArray(result?.statements) ? result.statements : [];
  for (const sectionItems of sections) {
    const statements = Array.isArray(sectionItems) ? sectionItems : [];
    for (const statement of statements) {
      const id = statement?.id;
      if (typeof id !== "string") {
        continue;
      }
      if (id.endsWith("profile_conformance")) {
        return {
          label: "Profile Conformance",
          value: statement?.value,
        };
      }
      if (id.endsWith("profile_compliance")) {
        return {
          label: "Profile Compliance",
          value: statement?.value,
        };
      }
    }
  }

  if (Object.prototype.hasOwnProperty.call(result || {}, "profile_conformance")) {
    return {
      label: "Profile Conformance",
      value: result.profile_conformance,
    };
  }

  if (Object.prototype.hasOwnProperty.call(result || {}, "profile_compliance")) {
    return {
      label: "Profile Compliance",
      value: result.profile_compliance,
    };
  }

  return null;
}

function createStatementCard(statement) {
  const card = document.createElement("article");
  card.className = "statement-card";

  const hasBoolValue = typeof statement?.value === "boolean";
  if (hasBoolValue) {
    card.classList.add(statement.value ? "bool-true" : "bool-false");
  }

  const header = document.createElement("div");
  header.className = "statement-header";

  const headerLeft = document.createElement("div");
  const id = document.createElement("div");
  id.className = "statement-id";
  id.textContent = statement?.id || "(no id)";
  headerLeft.appendChild(id);

  if (statement?.title) {
    const title = document.createElement("div");
    title.className = "statement-title";
    title.textContent = statement.title;
    headerLeft.appendChild(title);
  }

  header.appendChild(headerLeft);

  if (Object.prototype.hasOwnProperty.call(statement || {}, "value")) {
    const value = document.createElement("div");
    value.className = "statement-value";
    if (typeof statement.value === "boolean") {
      value.classList.add(statement.value ? "true" : "false");
      value.textContent = statement.value ? "✅ TRUE" : "❌ FALSE";
    } else {
      value.textContent = `ℹ️ ${formatValueText(statement.value)}`;
    }
    header.appendChild(value);
  }

  card.appendChild(header);

  if (statement?.report_text) {
    const report = document.createElement("p");
    report.className = "statement-report";
    report.textContent = statement.report_text;
    card.appendChild(report);
  }

  return card;
}

function renderVisualResult(result) {
  visualView.innerHTML = "";

  if (!result || typeof result !== "object") {
    const box = document.createElement("div");
    box.className = "error-box";
    box.textContent = "Result payload is not an object.";
    visualView.appendChild(box);
    return;
  }

  const summary = findProfileQualitySummary(result);
  if (summary) {
    visualView.appendChild(createConformanceSummary(summary.label, summary.value));
  }

  const sections = Array.isArray(result.statements) ? result.statements : [];
  if (sections.length === 0) {
    const box = document.createElement("div");
    box.className = "summary-card";
    box.textContent = "No statement sections returned.";
    visualView.appendChild(box);
    return;
  }

  sections.forEach((sectionItems, sectionIndex) => {
    const section = document.createElement("section");
    section.className = "section-block";

    const title = document.createElement("h3");
    title.className = "section-title";
    title.textContent = `Section ${sectionIndex + 1}`;
    section.appendChild(title);

    const statements = Array.isArray(sectionItems) ? sectionItems : [];
    statements.forEach((statement) => {
      section.appendChild(createStatementCard(statement));
    });

    visualView.appendChild(section);
  });
}

function renderError(errorMessage) {
  jsonTree.innerHTML = "";
  visualView.innerHTML = "";
  rawJsonView.innerHTML = "";
  const box = document.createElement("div");
  box.className = "error-box";
  box.textContent = errorMessage;
  jsonTree.appendChild(box);
  visualView.appendChild(box.cloneNode(true));
  rawJsonView.textContent = errorMessage;
}

function switchTab(tabName) {
  const visualActive = tabName === "visual";
  const jsonActive = tabName === "json";
  const rawActive = tabName === "raw-json";

  tabVisual.classList.toggle("active", visualActive);
  tabVisual.setAttribute("aria-selected", visualActive ? "true" : "false");
  tabJson.classList.toggle("active", jsonActive);
  tabJson.setAttribute("aria-selected", jsonActive ? "true" : "false");
  tabRawJson.classList.toggle("active", rawActive);
  tabRawJson.setAttribute("aria-selected", rawActive ? "true" : "false");

  visualView.classList.toggle("active", visualActive);
  jsonTree.classList.toggle("active", jsonActive);
  rawJsonView.classList.toggle("active", rawActive);
}

function setupVerticalResizer() {
  if (!verticalResizer || !inputsRow || !outputPanel) {
    return;
  }

  const appShell = document.querySelector(".app-shell");
  const titleRow = document.querySelector(".title-row");
  if (!appShell || !titleRow) {
    return;
  }

  const minTop = 180;
  const minBottom = 180;
  let startY = 0;
  let startTop = 0;
  let dragging = false;

  function onPointerMove(event) {
    if (!dragging) {
      return;
    }

    const shellRect = appShell.getBoundingClientRect();
    const titleRect = titleRow.getBoundingClientRect();
    const shellStyle = window.getComputedStyle(appShell);
    const gap = Number.parseFloat(shellStyle.rowGap || shellStyle.gap || "0") || 0;
    const availableHeight =
      shellRect.height - titleRect.height - verticalResizer.offsetHeight - gap * 2;

    const delta = event.clientY - startY;
    const requestedTop = startTop + delta;
    const maxTop = availableHeight - minBottom;
    const nextTop = Math.min(Math.max(requestedTop, minTop), maxTop);

    inputsRow.style.flex = `0 0 ${nextTop}px`;
    outputPanel.style.flex = "1 1 auto";
  }

  function onPointerUp() {
    dragging = false;
    verticalResizer.classList.remove("dragging");
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
  }

  verticalResizer.addEventListener("pointerdown", (event) => {
    if (window.matchMedia("(max-width: 960px)").matches) {
      return;
    }
    dragging = true;
    startY = event.clientY;
    startTop = inputsRow.getBoundingClientRect().height;
    verticalResizer.classList.add("dragging");
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
  });
}

const runtimePromise = createRuntime();

selectJsonButton.addEventListener("click", async () => {
  try {
    const runtime = await runtimePromise;
    const result = await runtime.selectFile("json");
    if (!result) {
      return;
    }
    sourceJsonField.value = result.contents;
    selectedJsonPath.textContent = result.path;
    setStatus("JSON file loaded");
  } catch (error) {
    setStatus(`Failed to load JSON file: ${String(error)}`, true);
  }
});

selectYamlButton.addEventListener("click", async () => {
  try {
    const runtime = await runtimePromise;
    const result = await runtime.selectFile("yaml");
    if (!result) {
      return;
    }
    profileYamlField.value = result.contents;
    yamlPath = runtime.kind === "tauri" ? result.path : null;
    selectedYamlPath.textContent = result.path;
    if (runtime.kind === "browser") {
      setStatus("YAML file loaded");
    } else {
      setStatus("YAML file loaded");
    }
  } catch (error) {
    setStatus(`Failed to load YAML file: ${String(error)}`, true);
  }
});

evaluateButton.addEventListener("click", async () => {
  const sourceJson = sourceJsonField.value.trim();
  const profileYaml = profileYamlField.value.trim();

  if (!sourceJson || !profileYaml) {
    setStatus("Both JSON and YAML fields are required", true);
    return;
  }

  try {
    const runtime = await runtimePromise;
    evaluateButton.disabled = true;
    setStatus("Evaluating...");

    const result = await runtime.evaluateProfile({
      sourceJson,
      profileYaml,
      profilePath: yamlPath,
    });

    renderVisualResult(result);
    renderJsonResult(result);
    renderRawJsonResult(result);
    switchTab("visual");
    setStatus("Evaluation complete");
  } catch (error) {
    renderError(String(error));
    switchTab("visual");
    setStatus("Evaluation failed", true);
  } finally {
    evaluateButton.disabled = false;
  }
});

tabVisual.addEventListener("click", () => switchTab("visual"));
tabJson.addEventListener("click", () => switchTab("json"));
tabRawJson.addEventListener("click", () => switchTab("raw-json"));

setupVerticalResizer();
