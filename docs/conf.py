import os.path

# Simulate import of the "neezer" package
supy_module_path = os.path.join(
    os.path.dirname(__file__), "..", "neezer", "__init__.py"
)
with open(supy_module_path, "rt", encoding="utf-8") as f:
    neezer = type("", (), {})()
    exec(f.read(), neezer.__dict__)


# -- Project information -----------------------------------------------------

project = neezer.NAME
author = neezer.AUTHOR
copyright = "2013-2023, " + author

version = neezer.VERSION
release = neezer.VERSION


# -- General configuration ---------------------------------------------------

extensions = []
templates_path = []
source_suffix = ".rst"
master_doc = "index"
exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]
smartquotes_action = "qe"

primary_domain = None
highlight_language = "none"

language = "en"


# -- Options for HTML output -------------------------------------------------

html_theme = "alabaster"
html_theme_options = {
    "description": neezer.DESCRIPTION,
    "github_user": "spl0k",
    "github_repo": "neezer",
    "github_type": "star",
}
html_static_path = ["_static"]

html_sidebars = {
    "*": [
        "about.html",
        "navigation.html",
        "relations.html",
        "searchbox.html",
        "donate.html",
    ],
    "setup/**": [
        "about.html",
        "localtoc.html",
        "navigation.html",
        "relations.html",
        "searchbox.html",
        "donate.html",
    ],
}

html_domain_indices = False


# -- Options for manual page output ------------------------------------------

_man_authors = ["Louis-Philippe Véronneau", author]

# (source start file, name, description, authors, manual section).
man_pages = [
    (
        "man/neezer-cli",
        "neezer-cli",
        "Neezer management command line interface",
        _man_authors,
        1,
    ),
    (
        "man/neezer-cli-user",
        "neezer-cli-user",
        "Neezer user management commands",
        _man_authors,
        1,
    ),
    (
        "man/neezer-cli-folder",
        "neezer-cli-folder",
        "Neezer folder management commands",
        _man_authors,
        1,
    ),
    (
        "man/neezer-daemon",
        "neezer-daemon",
        "Neezer background daemon",
        _man_authors,
        1,
    ),
    (
        "man/neezer-server",
        "neezer-server",
        "Python implementation of the Subsonic server API",
        [author],
        1,
    ),
]
