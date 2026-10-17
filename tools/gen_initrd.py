#!/usr/bin/env python3
"""Create a deterministic ustar image used as Horizon OS's C: drive fixture."""

import io
import sys
import tarfile


def add_directory(archive: tarfile.TarFile, path: str) -> None:
    """Add one deterministic directory entry to the read-only C: archive."""
    # NOTE: Open-source Contribution Area: preserve deterministic metadata if
    # the archive format or directory permissions are extended.
    entry = tarfile.TarInfo(path)
    entry.type = tarfile.DIRTYPE
    entry.mode = 0o755
    entry.uid = 0
    entry.gid = 0
    entry.uname = ""
    entry.gname = ""
    entry.mtime = 0
    archive.addfile(entry)


def add_file(archive: tarfile.TarFile, path: str, content: bytes) -> None:
    """Add a deterministic regular file, preserving the supplied bytes exactly."""
    # NOTE: Open-source Contribution Area: keep writes in the runtime overlay;
    # this generator should continue to produce an immutable base image.
    entry = tarfile.TarInfo(path)
    entry.type = tarfile.REGTYPE
    entry.mode = 0o644
    entry.uid = 0
    entry.gid = 0
    entry.uname = ""
    entry.gname = ""
    entry.mtime = 0
    entry.size = len(content)
    archive.addfile(entry, io.BytesIO(content))


def main(output: str, executable_fixture: str, wallpaper_fixture: str) -> None:
    """Build the VFS archive with PE and BMP fixtures under C:\\."""
    # NOTE: Open-source Contribution Area: keep generated executable fixtures
    # non-runnable until Horizon implements an isolated PE process loader.
    with tarfile.open(output, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for path in (
            "Documents/",
            "Apps/",
            "System/",
            "System/Trash/",
            "Windows/",
            "Windows/System/",
            "Users/",
            "Users/Guest/",
            "Users/Default/",
            "Users/Default/Documents/",
        ):
            add_directory(archive, path)
        add_file(
            archive,
            "README.TXT",
            b"Horizon OS virtual C drive.\nFiles are loaded from the GRUB initrd module.\n",
        )
        add_file(
            archive,
            "Documents/WELCOME.TXT",
            b"Welcome to Horizon OS.\nThis file is stored in the boot RAM disk.\n",
        )
        add_file(
            archive,
            "Apps/ABOUT.TXT",
            b"Applications are isolated by default.\n",
        )
        with open(executable_fixture, "rb") as fixture:
            add_file(archive, "Apps/TEST.EXE", fixture.read())
        with open(wallpaper_fixture, "rb") as fixture:
            add_file(archive, "Windows/System/wallpaper.bmp", fixture.read())
        add_file(
            archive,
            "System/INFO.TXT",
            b"Immutable system image fixture.\n",
        )
        add_file(
            archive,
            "Users/Guest/PROFILE.TXT",
            b"Guest user profile fixture.\n",
        )


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit(f"usage: {sys.argv[0]} OUTPUT PE_FIXTURE WALLPAPER_FIXTURE")
    main(sys.argv[1], sys.argv[2], sys.argv[3])
