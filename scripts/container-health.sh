#!/bin/sh
status=0
curl -fsS --max-time 4 -o /dev/null "http://127.0.0.1:${PORT:-3000}/api/heartbeat" || status=$?

awk -v status="$status" '
function field(file, key,   line, parts, value) {
  while ((getline line < file) > 0) {
    if (!key) { value = line; break }
    split(line, parts)
    if (parts[1] == key) { value = parts[2]; break }
  }
  close(file)
  return value
}
function mib(n, divisor) { return n ~ /^[0-9]+$/ ? sprintf("%.2f", n / divisor) : "null" }
BEGIN {
  for (i = 1; i < ARGC; i++) {
    name = field(ARGV[i] "/comm")
    if (name == "maap-server") rust = ARGV[i]
    if (name == "node" || name ~ /^next-server( |$)/) { node = ARGV[i]; nodes++ }
  }
  proc = (rust ? rust : node) "/status"
  # Resolve the current cgroup, not the host root when the cgroup namespace is shared.
  while ((getline line < "/proc/self/cgroup") > 0) {
    split(line, cg, ":")
    if (cg[1] == "0" && cg[2] == "") v2 = "/sys/fs/cgroup" cg[3]
    if (cg[2] ~ /(^|,)memory(,|$)/) v1 = "/sys/fs/cgroup/memory" cg[3]
  }
  if (v1 || v2) {
    dir = v1 ? v1 : v2
    usage = field(dir (v1 ? "/memory.usage_in_bytes" : "/memory.current"))
    limit = field(dir (v1 ? "/memory.limit_in_bytes" : "/memory.max"))
    inactive = field(dir "/memory.stat", v1 ? "total_inactive_file" : "inactive_file")
    # Docker stats excludes inactive file cache from the Linux working set.
    if (usage ~ /^[0-9]+$/ && inactive ~ /^[0-9]+$/)
      working = sprintf("%.0f", usage - (inactive + 0 < usage + 0 ? inactive : 0))
    if (v1 && limit + 0 >= 9e18) limit = "max"
  }
  printf "{\"ok\":%s,\"server\":%s,\"rss_mib\":%s,\"peak_rss_mib\":%s,\"swap_mib\":%s,\"node_processes\":%d,", \
    (status == 0 ? "true" : "false"), (rust ? "\"rust\"" : node ? "\"node\"" : "null"), \
    mib(field(proc, "VmRSS:"), 1024), mib(field(proc, "VmHWM:"), 1024), mib(field(proc, "VmSwap:"), 1024), nodes
  printf "\"container_working_set_mib\":%s,\"container_limit_mib\":%s,\"host_available_mib\":%s}\n", \
    mib(working, 1048576), mib(limit, 1048576), mib(field("/proc/meminfo", "MemAvailable:"), 1024)
  exit
}' /proc/[0-9]*

exit "$status"
