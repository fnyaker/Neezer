Neezer setup
===============

This guide details the required steps to get a Neezer instance ready to
start serving your music.

.. rubric:: TL;DR

For the impatient, here's a quick summary to get Neezer installed and ready
to start serving (but this doesn't create any user nor specifies where your
music is located 😏). This uses `gunicorn`__, but there are
:doc:`other options <deploying/index>`.

::

   pip install neezer
   pip install gunicorn
   neezer-server

__ https://gunicorn.org/

.. rubric:: Table of contents

.. toctree::
   :maxdepth: 2

   install
   database
   configuration
   deploying/index
   daemon

.. _docker:

.. rubric:: Docker

Another solution rather than going through the whole setup process yourself is
to use a ready-to-use Docker image. While we don't provide images for Neezer,
that didn't keep the community from creating some. Take a look on the
`Docker Hub`__ and pick one you like. For more details on their usage, please
refer to the readme of said images.

__ https://hub.docker.com/search?q=neezer&type=image
