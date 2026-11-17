# home-cc machine stats, Linux. POSIX sh only (dash, busybox ash); needs awk, df, sort, uname.
# Sent on stdin to `sh -s`, so no quoting layer and no dependence on the login shell.
LC_ALL=C; export LC_ALL
PATH="$PATH:/usr/sbin:/sbin:/usr/bin:/bin"; export PATH
TAB=$(printf '\t')
echo '@@hcc-stats 1'
if [ -r /etc/os-release ]; then
  ( . /etc/os-release
    printf 'os=%s\nos_id=%s\nos_version=%s\n' "${PRETTY_NAME:-${NAME:-Linux}}" "${ID:-linux}" "${VERSION_ID:-}" )
else
  printf 'os=%s\n' "$(uname -s)"
fi
printf 'kernel=%s\narch=%s\nhost=%s\n' "$(uname -r)" "$(uname -m)" "$(uname -n)"
if command -v pveversion >/dev/null 2>&1; then
  printf 'pve=%s\n' "$(pveversion 2>/dev/null </dev/null | head -n 1)"
fi
read -r up _rest < /proc/uptime && echo "uptime_s=${up%%.*}"
read -r l1 l5 l15 _rest < /proc/loadavg && echo "load=$l1 $l5 $l15"
# cpu line: user nice system idle iowait irq softirq steal (guest is already inside user).
# %.0f, not %d: old mawk clamps %d at 2^31-1.
awk '$1 == "cpu" { t = 0; for (i = 2; i <= 9; i++) t += $i; printf "cpu_ticks=%.0f %.0f\n", t, $5 + $6 }
     /^cpu[0-9]/ { n++ }
     END { printf "ncpu=%d\n", n }' /proc/stat
awk '$1 == "MemTotal:" { t = $2 } $1 == "MemAvailable:" { a = $2; ha = 1 }
     $1 == "MemFree:" { f = $2 } $1 == "Buffers:" { b = $2 } $1 == "Cached:" { c = $2 }
     $1 == "SwapTotal:" { st = $2 } $1 == "SwapFree:" { sf = $2 }
     END { if (!ha) a = f + b + c
           printf "mem_kb=%.0f %.0f\nswap_kb=%.0f %.0f\n", t, a, st, sf }' /proc/meminfo
if [ -r /proc/spl/kstat/zfs/arcstats ]; then
  awk '$1 == "size" { printf "zfs_arc_kb=%.0f\n", $3 / 1024 }' /proc/spl/kstat/zfs/arcstats
fi
# Real, local filesystems only, chosen by type from /proc/mounts (an allow-list: tmpfs,
# overlay, squashfs, proc, nfs, cifs and anything new and virtual are skipped, and a dead
# network mount can never hang df because df is only run on the mounts picked here).
# One line per device (bind mounts and btrfs subvolumes repeat the device): the shortest
# mount point that is a folder wins. Single files mounted from elsewhere, such as
# /etc/hosts inside a container, are not disks the owner would recognise.
awk '
function unesc(s,   out, i, o) {   # /proc/mounts writes space, tab, newline, backslash as \040 \011 \012 \134
  out = ""
  while ((i = index(s, "\\")) > 0) {
    o = substr(s, i + 1, 3)
    if (o ~ /^[0-7][0-7][0-7]$/) {
      out = out substr(s, 1, i - 1) sprintf("%c", substr(o, 1, 1) * 64 + substr(o, 2, 1) * 8 + substr(o, 3, 1))
      s = substr(s, i + 4)
    } else { out = out substr(s, 1, i); s = substr(s, i + 1) }
  }
  return out s
}
$3 ~ /^(ext[234]|xfs|btrfs|zfs|f2fs|vfat|exfat|ntfs|ntfs3|fuseblk|jfs|reiserfs|bcachefs|ufs|hfsplus)$/ {
  dev = unesc($1); mp = unesc($2)
  if (dev ~ /[\t\n]/ || mp ~ /[\t\n]/) next
  printf "%d\t%s\t%s\t%s\n", length(mp), $3, dev, mp
}
' /proc/mounts | sort -n |
while IFS="$TAB" read -r len fs dev mp; do
  [ -d "$mp" ] || continue
  case "$seen" in *"$TAB$dev$TAB"*) continue ;; esac
  seen="$seen$TAB$dev$TAB"
  # POSIX df line: device, 1024-blocks, used, available, capacity%, mount point.
  # Device and mount point may contain spaces, so pick the numbers out by pattern.
  nums=$(df -Pk -- "$mp" 2>/dev/null | awk '
    NR == 2 && match($0, / [0-9]+ +[0-9]+ +[0-9]+ +[0-9]+%/) {
      split(substr($0, RSTART + 1, RLENGTH - 1), a, / +/); printf "%s\t%s\t%s", a[1], a[2], a[3] }')
  [ -n "$nums" ] && printf 'disk=%s\t%s\t%s\t%s\n' "$nums" "$fs" "$dev" "$mp"
done
echo '@@end'
exit 0
