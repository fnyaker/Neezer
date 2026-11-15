export const MIN_PASSWORD = 12;
export const MAX_PASSWORD = 128;
export const MIN_USERNAME = 3;
export const MAX_USERNAME = 32;
export const DRAFT_KEY = "nsInvite.draft";
export const DRAFT_TTL = 10 * 60 * 1000;
export const TOKEN_RE = /^[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{24}$/;

const ALLOWED = /^[a-z0-9_.-]+$/;
const EDGES = /^[a-z0-9][a-z0-9_.-]*[a-z0-9]$/;

const charCount = (value) => [...String(value || "")].length;

function store(kind) {
  try {
    return globalThis[kind] || null;
  } catch {
    return null;
  }
}

export function foldName(raw) {
  return String(raw == null ? "" : raw)
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase();
}

export function foldUsername(raw) {
  return foldName(raw).trim();
}

export function refusedCharacters(raw) {
  const seen = [];
  for (const char of foldUsername(raw)) {
    if (ALLOWED.test(char) || seen.includes(char)) continue;
    seen.push(char);
  }
  return seen;
}

function charLabel(char) {
  if (char === " ") return "l'espace";
  if (char === "\t") return "la tabulation";
  if (char === "\n") return "le retour à la ligne";
  return `« ${char} »`;
}

export function usernameProblem(raw) {
  const folded = foldUsername(raw);
  const length = charCount(folded);
  if (!length) return "too_short";
  if (length < MIN_USERNAME) return "too_short";
  if (length > MAX_USERNAME) return "too_long";
  if (!ALLOWED.test(folded)) return "charset";
  if (!EDGES.test(folded)) return "edge";
  return null;
}

export function usernameMessage(raw) {
  const problem = usernameProblem(raw);
  if (problem === "too_short") return `Il faut au moins ${MIN_USERNAME} caractères.`;
  if (problem === "too_long") return `${MAX_USERNAME} caractères au plus.`;
  if (problem === "edge") return "Le nom doit commencer et finir par une lettre ou un chiffre.";
  if (problem === "charset") {
    const bad = refusedCharacters(raw);
    const list = bad.slice(0, 3).map(charLabel).join(", ");
    const head =
      bad.length > 1
        ? "Ces caractères ne sont pas autorisés"
        : "Ce caractère n'est pas autorisé";
    return `${head} : ${list}. Utilise a–z, 0–9, le point, le tiret ou le tiret bas.`;
  }
  return "";
}

export function passwordScore(password, username = "") {
  const value = String(password || "");
  const length = charCount(value);
  if (!length) return 0;
  if (length < MIN_PASSWORD) return 1;
  let score = length >= 24 ? 4 : length >= 16 ? 3 : 2;
  const name = foldUsername(username);
  if (name) {
    const folded = foldUsername(value);
    const reversed = [...name].reverse().join("");
    if (folded.includes(name) || folded.includes(reversed)) score = Math.min(score, 2);
  }
  if (/^(.)\1{7,}$/.test(value)) score = Math.min(score, 2);
  return score;
}

export function passwordLabel(password, username = "") {
  if (charCount(password) < MIN_PASSWORD) return "Trop court";
  return ["", "Faible", "Correct", "Bon", "Très bon"][passwordScore(password, username)];
}

export function passwordProblem(password, username = "") {
  const value = String(password || "");
  const length = charCount(value);
  if (length < MIN_PASSWORD) return "short";
  if (length > MAX_PASSWORD) return "long";
  const name = foldUsername(username);
  if (name) {
    const folded = foldUsername(value);
    const reversed = [...name].reverse().join("");
    if (folded.includes(name) || folded.includes(reversed)) return "same_as_username";
  }
  return null;
}

export function passwordMessage(password, username = "") {
  const problem = passwordProblem(password, username);
  if (problem === "short") return `${MIN_PASSWORD} caractères minimum.`;
  if (problem === "long") return `${MAX_PASSWORD} caractères au plus.`;
  if (problem === "same_as_username")
    return "Le mot de passe ne peut pas contenir ton nom d'utilisateur.";
  return "";
}

export function tokenShapeOk(token) {
  return (
    typeof token === "string" &&
    token.length === 47 &&
    TOKEN_RE.test(token)
  );
}

export function expiryLabel(iso) {
  if (!iso) return "";
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  try {
    const text = new Intl.DateTimeFormat("fr-FR", {
      day: "numeric",
      month: "long",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(at);
    return `Expire le ${text}.`;
  } catch {
    return "";
  }
}

export function landingFor(account) {
  const step = account && typeof account.onboarding === "string" ? account.onboarding : "";
  return step && step !== "done" ? "/onboarding" : "/";
}

export function newRequestId() {
  try {
    if (globalThis.crypto && typeof globalThis.crypto.randomUUID === "function") {
      return globalThis.crypto.randomUUID();
    }
  } catch {
  }
  try {
    if (globalThis.crypto && typeof globalThis.crypto.getRandomValues === "function") {
      const bytes = new Uint8Array(16);
      globalThis.crypto.getRandomValues(bytes);
      return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
    }
  } catch {
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

export function rememberDraft({ token, name = "", request_id }, storage = store("sessionStorage")) {
  if (!storage || !token || !request_id) return false;
  try {
    storage.setItem(
      DRAFT_KEY,
      JSON.stringify({ token, name: String(name || ""), request_id, at: Date.now() })
    );
    return true;
  } catch {
    return false;
  }
}

export function readDraft(token, storage = store("sessionStorage")) {
  if (!storage || !token) return null;
  try {
    const raw = JSON.parse(storage.getItem(DRAFT_KEY) || "null");
    if (!raw || raw.token !== token) return null;
    if (typeof raw.request_id !== "string" || !raw.request_id) return null;
    if (typeof raw.at !== "number" || Date.now() - raw.at > DRAFT_TTL) return null;
    return {
      request_id: raw.request_id,
      name: typeof raw.name === "string" ? raw.name : "",
    };
  } catch {
    return null;
  }
}

export function clearDraft(storage = store("sessionStorage")) {
  if (!storage) return;
  try {
    storage.removeItem(DRAFT_KEY);
  } catch {
  }
}
