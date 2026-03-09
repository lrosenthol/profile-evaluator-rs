const sourceJsonField = document.getElementById("source-json");
const profileYamlField = document.getElementById("profile-yaml");
const evaluateButton = document.getElementById("evaluate-button");
const jsonTree = document.getElementById("json-tree");
const status = document.getElementById("status");

const selectJsonButton = document.getElementById("select-json");
const selectYamlButton = document.getElementById("select-yaml");
const selectedJsonPath = document.getElementById("selected-json-path");
const selectedYamlPath = document.getElementById("selected-yaml-path");

let yamlPath = null;

function setStatus(message, isError = false) {
  status.textContent = message;
  status.classList.toggle("error", isError);
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

function renderError(errorMessage) {
  jsonTree.innerHTML = "";
  const box = document.createElement("div");
  box.className = "error-box";
  box.textContent = errorMessage;
  jsonTree.appendChild(box);
}

async function selectFile(kind) {
  return window.__TAURI__.core.invoke("select_and_load_file", { kind });
}

selectJsonButton.addEventListener("click", async () => {
  try {
    const result = await selectFile("json");
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
    const result = await selectFile("yaml");
    if (!result) {
      return;
    }
    profileYamlField.value = result.contents;
    yamlPath = result.path;
    selectedYamlPath.textContent = result.path;
    setStatus("YAML file loaded");
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
    evaluateButton.disabled = true;
    setStatus("Evaluating...");

    const result = await window.__TAURI__.core.invoke("evaluate_profile", {
      sourceJson,
      profileYaml,
      profilePath: yamlPath,
    });

    renderJsonResult(result);
    setStatus("Evaluation complete");
  } catch (error) {
    renderError(String(error));
    setStatus("Evaluation failed", true);
  } finally {
    evaluateButton.disabled = false;
  }
});
