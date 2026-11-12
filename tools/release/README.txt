teta-wot {version} for Windows x86_64
=====================================

teta-wot serves laboratory hardware and software as W3C Web of Things Things.
This package holds its demo: a simulated microscope (a stage, a camera with a
live MJPEG preview, and an autofocus), built for Windows x86_64. To build your
own Things, use the source (https://github.com/lucaswewa/teta-wot) and its guide.

simulated-microscope.exe
    The microscope as a Thing server:

        simulated-microscope.exe -c microscope.json

    Then open http://127.0.0.1:5000/docs for the interactive API, or
    http://127.0.0.1:5000/camera/preview/viewer for the live preview.
    Without arguments it runs a short demonstration in-process and exits.

microscope-service.exe
    The same server as a Windows service (from an administrator console):

        microscope-service.exe install --port 5000
        Start-Service wot-microscope
        Stop-Service wot-microscope
        microscope-service.exe uninstall

    It writes its configuration and log next to itself.

CHANGELOG.md             what changed in this release
THIRD_PARTY_LICENSES.txt the licences of the crates compiled into the binaries
THIRD_PARTY_NOTICES.md   the files copied into teta-wot's sources

teta-wot is under the MIT licence. The binaries contain code from other
crates, each under its own licence (THIRD_PARTY_LICENSES.txt). This software
is based in part on the work of the Independent JPEG Group: the JPEG encoder
of the camera's preview and captures.
