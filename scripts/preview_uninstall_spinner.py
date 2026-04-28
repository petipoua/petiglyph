#!/usr/bin/env python3

import itertools
import sys
import time


SPINNERS = [
    ("circleHalves", ["◐", "◓", "◑", "◒"]),
    ("circleQuarters", ["◴", "◷", "◶", "◵"]),
    ("arc", ["◜", "◠", "◝", "◞", "◡", "◟"]),
    ("circle", ["◡", "⊙", "◠"]),
    ("balloon2", [".", "o", "O", "°", "O", "o", "."]),
    ("dots11", ["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"]),
]
PER_SPINNER_DURATION_SECONDS = 4
FRAME_DELAY_SECONDS = 0.165


def preview_spinner(name: str, frames: list[str]) -> None:
    sys.stdout.write(f"\n{name}\n")
    sys.stdout.flush()

    end = time.time() + PER_SPINNER_DURATION_SECONDS
    for frame in itertools.cycle(frames):
        sys.stdout.write(f"\r\033[31;1m{frame}\033[0m Removing...   ")
        sys.stdout.flush()
        time.sleep(FRAME_DELAY_SECONDS)
        if time.time() >= end:
            break

    sys.stdout.write("\r                    \r")
    sys.stdout.flush()


def main() -> int:
    try:
        for name, frames in SPINNERS:
            preview_spinner(name, frames)
    except KeyboardInterrupt:
        sys.stdout.write("\n")
        sys.stdout.flush()
        return 130

    sys.stdout.write("Done.\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
