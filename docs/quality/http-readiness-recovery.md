# HTTP readiness recovery

Gate run run-7ab1480e8372 rejected GH-16 head
9c0447b3acf0c771b7f37d4bf8dfa7d99f31cd2d with a server readiness timeout.
The previous reader mixed selectors with buffered TextIOWrapper.readline().
A single OS write containing an ordinary startup line followed by the readiness
line leaves the latter inside Python's buffer; selectors no longer reports the
OS pipe readable. A standalone reproduction confirmed the old reader timed out
although the readiness line had already been emitted. The old host did not
retain server logs, so the precise original startup sequence cannot be recovered.

The replacement reads OS pipe chunks and parses complete lines itself. It
retains http-server.stdout and http-server.stderr and reports their locations on
failure. The automatic diagnostic exporter includes these files. The readiness
timeout remains 20 seconds; failed startup is still rejected.

Four regression tests cover multiple lines in one write, a line split across
writes, early process exit and a real timeout. This change affects trusted capture
code and therefore requires fresh complete Gate acceptance and adoption, not a
waiver of runtime pins or measurement series.
