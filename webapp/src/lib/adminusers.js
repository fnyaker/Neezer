export const PAGE_SIZE = 25;
export const MAX_ROWS = 250;

export const FILTERS = [
  { id: "all", label: "Tous" },
  { id: "active", label: "Actifs" },
  { id: "invited", label: "Invités" },
  { id: "admins", label: "Administrateurs" },
  { id: "members", label: "Membres" },
  { id: "disabled", label: "Désactivés" },
];

const DATE = new Intl.DateTimeFormat("fr-FR", {
  day: "numeric",
  month: "short",
  year: "numeric",
});
const DATETIME = new Intl.DateTimeFormat("fr-FR", {
  dateStyle: "medium",
  timeStyle: "short",
});
const CLOCK = new Intl.DateTimeFormat("fr-FR", {
  hour: "2-digit",
  minute: "2-digit",
});

const REFUSALS = {
  last_admin: "Impossible : c'est le dernier administrateur actif.",
  self_delete: "Vous ne pouvez pas supprimer votre propre compte.",
  self_disable: "Vous ne pouvez pas désactiver votre propre compte.",
  server_account_holder:
    "Ce compte porte le compte Deezer du serveur. Changez-en le titulaire dans Réglages → Compte Deezer avant de le supprimer.",
  forbidden: "Votre compte n'a plus les droits d'administration.",
  not_found: "Ce compte n'existe plus.",
  invite_not_found: "Cette invitation n'existe plus.",
  invite_used: "Cette invitation a déjà servi.",
  invite_invalid: "Cette invitation n'est plus valide.",
  rate_limited: "Trop de tentatives. Réessayez dans quelques minutes.",
  invalid_note: "La note doit faire 64 caractères au plus.",
  invalid_expires_in: "La durée doit être comprise entre 1 heure et 30 jours.",
  too_large: "La demande est trop volumineuse.",
  server_error: "Erreur du serveur. Réessayez.",
};

export function foldText(value) {
  return String(value ?? "")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .trim();
}

export function safeDate(iso) {
  if (!iso) return null;
  const at = Date.parse(iso);
  return Number.isNaN(at) ? null : new Date(at);
}

export function shortDate(iso) {
  const date = safeDate(iso);
  return date ? DATE.format(date) : "—";
}

export function absoluteTime(iso) {
  const date = safeDate(iso);
  return date ? DATETIME.format(date).replace(",", " à") : "—";
}

export function clockTime(iso) {
  const date = safeDate(iso);
  return date ? CLOCK.format(date) : "—";
}

export function relativeTime(iso, now = Date.now()) {
  const date = safeDate(iso);
  if (!date) return "—";
  const seconds = Math.max(0, Math.floor((now - date.getTime()) / 1000));
  if (seconds < 60) return "à l'instant";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `il y a ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `il y a ${hours} h`;
  const days = Math.floor(hours / 24);
  if (days < 31) return `il y a ${days} j`;
  const months = Math.floor(days / 30);
  if (months < 12) return `il y a ${months} mois`;
  const years = Math.floor(months / 12);
  return `il y a ${years} an${years > 1 ? "s" : ""}`;
}

export function expiryLabel(iso, now = Date.now()) {
  const date = safeDate(iso);
  if (!date) return "—";
  const left = date.getTime() - now;
  if (left <= 0) return "expirée";
  const minutes = Math.ceil(left / 60000);
  if (minutes < 60) return `expire dans ${minutes} min`;
  const hours = Math.ceil(left / 3600000);
  if (hours < 48) return `expire dans ${hours} h`;
  const days = Math.ceil(left / 86400000);
  if (days < 31) return `expire dans ${days} j`;
  const months = Math.ceil(days / 30);
  return `expire dans ${months} mois`;
}

export function accountRow(user) {
  return {
    ...user,
    kind: "user",
    admin: !!user.admin,
    disabled: !!user.disabled,
    last_login: user.last_login || null,
  };
}

export function inviteRow(invite, now = Date.now()) {
  const note = typeof invite.note === "string" ? invite.note.trim() : "";
  return {
    kind: "invite",
    id: String(invite.id),
    name: note || "Invitation sans note",
    note,
    created: invite.created || null,
    expires: invite.expires || null,
    state: invite.state || "pending",
    created_by: invite.created_by || null,
    used_by: invite.used_by || null,
    used_at: invite.used_at || null,
    admin: false,
    disabled: false,
    last_login: null,
    expired: stateLabel({ kind: "invite", state: invite.state, expires: invite.expires }, now) === "Expirée",
  };
}

export function stateKey(row, now = Date.now()) {
  if (row.kind === "invite") {
    if (row.state === "used") return "used";
    if (row.state === "revoked") return "revoked";
    if (row.state === "expired" || row.expired) return "expired";
    if (expiryLabel(row.expires, now) === "expirée") return "expired";
    return "invite";
  }
  return row.disabled ? "disabled" : "active";
}

export function stateLabel(row, now = Date.now()) {
  const key = stateKey(row, now);
  if (key === "invite") return "Invité";
  if (key === "expired") return "Expirée";
  if (key === "used") return "Utilisée";
  if (key === "revoked") return "Révoquée";
  return key === "disabled" ? "Désactivé" : "Actif";
}

export function roleLabel(row) {
  return row.admin ? "Administrateur" : "Membre";
}

export function lastSeenLabel(row, now = Date.now()) {
  if (row.kind === "invite") {
    return row.expires ? `Expire le ${shortDate(row.expires)}` : "—";
  }
  return row.last_login ? relativeTime(row.last_login, now) : "—";
}

export function countLabel(count, singular, plural = `${singular}s`) {
  return `${count} ${count > 1 ? plural : singular}`;
}

export function inviteUrl(token, locationLike = null) {
  const at =
    locationLike || (typeof window !== "undefined" ? window.location : null);
  if (!at || !token) return "";
  return `${at.origin}${at.pathname}#/invite/${encodeURIComponent(token)}`;
}

export function mergeUsers(current, page, append = false) {
  const base = append && current ? current.rows : [];
  const rows = base.slice();
  const seen = new Set(rows.map((row) => row.id));
  for (const user of page?.users || []) {
    if (!user || user.id == null || seen.has(user.id)) continue;
    seen.add(user.id);
    rows.push(accountRow(user));
  }
  const total = Number(page?.total);
  return {
    rows,
    next: page?.next ?? null,
    total: Number.isFinite(total) ? total : rows.length,
  };
}

export function filterRows(rows, filter = "all", term = "") {
  const needle = foldText(term);
  return rows.filter((row) => {
    if (filter === "invited" && row.kind !== "invite") return false;
    if (filter === "active" && (row.kind !== "user" || row.disabled)) return false;
    if (filter === "disabled" && (row.kind !== "user" || !row.disabled)) return false;
    if (filter === "admins" && (row.kind !== "user" || !row.admin)) return false;
    if (filter === "members" && (row.kind !== "user" || row.admin)) return false;
    if (needle && row.kind === "invite") {
      return foldText(`${row.note || ""} ${row.name || ""}`).includes(needle);
    }
    return true;
  });
}

export function guardMessage(err) {
  const code = err?.body?.code || err?.code;
  if (code === "rate_limited" && err?.body?.reason === "invite_cap") {
    return "Trop d'invitations en attente. Révoquez-en ou attendez leur expiration.";
  }
  if (code && REFUSALS[code]) return REFUSALS[code];
  if (err?.status === 0 || err?.offline) return "Le serveur ne répond pas (réseau).";
  if (err?.status === 401) return "Session expirée : reconnectez-vous.";
  if (err?.status === 403) return REFUSALS.forbidden;
  if (err?.status === 404) return "Cet élément n'existe plus.";
  if (err?.status === 409) return "Cette action est impossible dans l'état actuel.";
  if (err?.status === 429) return REFUSALS.rate_limited;
  if (err?.status >= 500) return REFUSALS.server_error;
  const text = err?.body?.error;
  return typeof text === "string" && text ? text : "Cette action a échoué.";
}

export async function copyText(text) {
  const value = String(text || "");
  if (!value) return false;
  try {
    if (navigator?.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {}
  try {
    const field = document.createElement("textarea");
    field.value = value;
    field.setAttribute("readonly", "");
    field.style.position = "fixed";
    field.style.top = "-1000px";
    field.style.opacity = "0";
    document.body.appendChild(field);
    field.select();
    const copied = document.execCommand("copy");
    field.remove();
    return copied;
  } catch {
    return false;
  }
}
