// WHICH worlds exist, what each one is called and what it looks like —
// without the code that draws them.
//
// Three consumers need this and none of them should pay for a shader:
// `lib/viz/skins.js` has to know a world id is real (a skin naming a world that
// does not exist would render nothing), the settings panel shows the gallery
// and names the world the engine resolved, and the tests check that the
// catalogue and the loader agree. The GLSL itself is code-split one chunk per
// world (./index.js) and fetched the first time that world is on screen.
//
// `group` is the gallery's shelf. The name is all a person gets: the preview
// above the grid shows what the world looks like.
//
// MAINTAINING THIS: a world needs a row here AND a loader in ./index.js. The
// test suite fails if the two disagree.

export const GROUPS = [
  { id: "hard", label: "Hard" },
  { id: "techno", label: "Techno & machines" },
  { id: "trance", label: "Trance & psy" },
  { id: "bass", label: "Bass & breaks" },
  { id: "urban", label: "Urbain" },
  { id: "groove", label: "Pop & groove" },
  { id: "rock", label: "Rock & metal" },
  { id: "calm", label: "Calme" },
  { id: "world", label: "Monde" },
  { id: "retro", label: "Rétro" },
  { id: "modes", label: "Classiques" },
];

export const WORLD_META = {
  // --- hard ------------------------------------------------------------------
  forge: { group: "hard", label: "Forge" },
  cymatics: { group: "hard", label: "Cymatique" },
  shatter: { group: "hard", label: "Éclats" },
  lasers: { group: "hard", label: "Lasers" },
  bounce: { group: "hard", label: "Rebond" },
  saw: { group: "hard", label: "Scie" },
  stairs: { group: "hard", label: "Arpège" },
  pingpong: { group: "hard", label: "Ping-pong" },
  soundsystem: { group: "hard", label: "Sound system" },
  static: { group: "hard", label: "Parasites" },
  microwave: { group: "hard", label: "Micro-ondes" },
  fireworks: { group: "hard", label: "Feu d'artifice" },

  // --- techno & machines -------------------------------------------------------
  tunnel: { group: "techno", label: "Corridor" },
  warehouse: { group: "techno", label: "Hangar" },
  ridges: { group: "techno", label: "Crêtes" },
  lattice: { group: "techno", label: "Treillis" },
  circuit: { group: "techno", label: "Circuit" },

  // --- trance & psy ------------------------------------------------------------
  hyperspace: { group: "trance", label: "Hyperespace" },
  kaleido: { group: "trance", label: "Kaléidoscope" },
  galaxy: { group: "trance", label: "Galaxie" },
  flow: { group: "trance", label: "Flux" },

  // --- bass & breaks -----------------------------------------------------------
  wobble: { group: "bass", label: "Wobble" },
  chrome: { group: "bass", label: "Chrome" },
  slices: { group: "bass", label: "Découpe" },

  // --- urbain -----------------------------------------------------------------
  vinyl: { group: "urban", label: "Vinyle" },
  halo: { group: "urban", label: "Halo" },
  nightdrive: { group: "urban", label: "Route de nuit" },
  neon: { group: "urban", label: "Néon" },

  // --- pop & groove ------------------------------------------------------------
  bokeh: { group: "groove", label: "Bokeh" },
  discoball: { group: "groove", label: "Boule à facettes" },
  silk: { group: "groove", label: "Soie" },
  artwork: { group: "groove", label: "Pochette vivante" },
  plasma: { group: "groove", label: "Lampe à plasma" },

  // --- rock & metal ------------------------------------------------------------
  stage: { group: "rock", label: "Scène" },
  inferno: { group: "rock", label: "Brasier" },
  storm: { group: "rock", label: "Orage" },

  // --- calme -----------------------------------------------------------------
  nebula: { group: "calm", label: "Nébuleuse" },
  aurora: { group: "calm", label: "Aurore" },
  piano: { group: "calm", label: "Piano" },
  ocean: { group: "calm", label: "Océan" },
  cathedral: { group: "calm", label: "Nef" },
  ink: { group: "calm", label: "Encre" },
  rain: { group: "calm", label: "Pluie" },
  fireflies: { group: "calm", label: "Lucioles" },

  // --- monde -----------------------------------------------------------------
  carnival: { group: "world", label: "Carnaval" },
  tropics: { group: "world", label: "Tropiques" },

  // --- rétro -----------------------------------------------------------------
  horizon: { group: "retro", label: "Horizon" },
  pixels: { group: "retro", label: "Pixels" },

  // --- classiques (also the fixed modes) --------------------------------------
  pulse: { group: "modes", label: "Pulsations" },
  spectrum: { group: "modes", label: "Spectre" },
};

/** When nothing names a genre at all, the archetype is still a better guess
 * than one fixed default. Genre -> world is decided by `lib/viz/skins.js`; this
 * is only the floor under it. */
const ARCHETYPE_WORLD = {
  sustain: "nebula",
  voice: "bokeh",
  groove: "tunnel",
  hard: "lasers",
  rock: "stage",
};

export function worldFor(archetype) {
  return ARCHETYPE_WORLD[archetype] || "bokeh";
}
