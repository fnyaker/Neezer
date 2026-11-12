neezer-cli-folder
====================

SYNOPSIS
--------

neezer-cli folder *--help*

neezer-cli folder **list**

neezer-cli folder **add** <*name*> <*path*>

neezer-cli folder **delete** <*name*>

neezer-cli folder **scan** [*--force*] [*--background* | *--foreground*] <*name*>

DESCRIPTION
-----------

The **neezer-cli folder** subcommand manages your library folders, where the
audio files are located. This allows one to list, add, delete and scan the
folders.

ARGUMENTS
---------

**list**
    List all the folders.

**add** <*name*> <*path*>
    Add a new library folder called <*name*> and located at <*path*>. <*name*>
    must be unique and <*path*> pointing to an existing directory. If
    ``neezer-daemon`` is running it will start to listen for changes in this
    folder but will not scan files already present in the folder.

**delete** <*name*>
    Delete the folder called <*name*>.

**scan** [*--force*] [*--background* | *--foreground*] <*name*>
    Scan the specified folders. If none is given, all the registered folders
    are scanned.

OPTIONS
-------

**-h**, **--help**
    Shows help and exits. Depending on where this option appears it will either
    list the available commands or display help for a specific command.

**-f**, **--force**
    Force scan of already known files even if they haven't changed. Might be
    useful if an update to neezer adds new metadata to audio files.

**--background**
    Scan in the background. Requires the ``neezer-daemon`` to be running.

**--foreground**
    Scan in the foreground, blocking the process while the scan is running.

If neither **--background** nor **--foreground** is provided, neezer-cli
will try to connect to the daemon to initiate a background scan, falling back
to a foreground scan if it isn't available.

EXAMPLES
--------

To add a new folder to your music library, you can do something like this::

   $ neezer-cli folder add MyLibrary /home/username/Music

Once you've added a folder, you will need to scan it::

   $ neezer-cli folder scan MyLibrary

The audio files residing in ``/home/username/Music`` will now appear under the
``MyLibrary`` folder on the clients.

SEE ALSO
--------

``neezer-cli (1)``, ``neezer-cli-user (1)``,
``neezer-server (1)``, ``neezer-daemon (1)``
