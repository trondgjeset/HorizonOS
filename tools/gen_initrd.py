#!/usr/bin/env python3
"""Create a deterministic ustar image used as Horizon OS's C: drive fixture."""

import io
import sys
import tarfile


def add_directory(archive: tarfile.TarFile, path: str) -> None:
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


def main(output: str) -> None:
    with tarfile.open(output, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for path in ("Documents/", "Apps/", "System/", "Users/", "Users/Guest/"):
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
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} OUTPUT")
    main(sys.argv[1])
