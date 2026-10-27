"""Test soak.py's CPU accounting without a running server."""

import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("soak", Path(__file__).parents[1] / "soak.py")
soak = importlib.util.module_from_spec(spec)
spec.loader.exec_module(soak)


def stat(utime=0, stime=0, cutime=0, cstime=0):
    # Fields after the command name; index 11 is utime.
    fields = ["0"] * 20
    fields[11:15] = [str(utime), str(stime), str(cutime), str(cstime)]
    return fields


class CpuFieldsTest(unittest.TestCase):
    def test_child_reaped_between_reads_is_counted_once(self):
        # The server reaps ffmpeg (6000 ticks) after the first read of its
        # own stat and before the child's stat is read.
        reads = {1: [stat(cutime=0), stat(cutime=6000), stat(cutime=6000), stat(cutime=6000)],
                 2: [FileNotFoundError, FileNotFoundError]}

        def read_stat(pid):
            value = reads[pid].pop(0)
            if value is FileNotFoundError:
                raise FileNotFoundError(pid)
            return value

        fields, live = soak.cpu_fields(1, [2], read_stat)
        self.assertEqual(int(fields[13]) + live, 6000)

    def test_child_reaped_after_its_read_is_not_counted_twice(self):
        # The child's stat still shows its 6000 ticks, then the server
        # reaps it before the second read of the server's stat.
        reads = {1: [stat(), stat(cutime=6000), stat(cutime=6000), stat(cutime=6000)],
                 2: [stat(utime=6000), FileNotFoundError]}

        def read_stat(pid):
            value = reads[pid].pop(0)
            if value is FileNotFoundError:
                raise FileNotFoundError(pid)
            return value

        fields, live = soak.cpu_fields(1, [2], read_stat)
        self.assertEqual(int(fields[13]) + live, 6000)

    def test_live_child_is_counted(self):
        reads = {1: [stat(), stat()], 2: [stat(utime=300, stime=100)]}
        fields, live = soak.cpu_fields(1, [2], lambda pid: reads[pid].pop(0))
        self.assertEqual((int(fields[13]), live), (0, 400))


unittest.main()
