#!/usr/bin/env bash
#
# Shared classpath construction for the scripts that need to compile against the
# server's own jars.
#
# Why this exists: the NeoForge install tree contains more than one version of some
# libraries. Guava is the one that bit us — 20.0 and 32.1.2-jre are both installed, and
# guava 20 does not have `ImmutableList.toImmutableList()`, which Minecraft's static
# initialisers call. `find` returns those two in filesystem order, so which one won
# depended on the order the directories happened to be created in.
#
# The result was that parity passed on one machine and failed on another with
# NoSuchMethodError, from identical inputs. Sorting the paths does not fix it either:
# "20.0" sorts before "32.1.2-jre", so a sorted classpath deterministically picks the
# wrong one.
#
# So: group by artifact directory, keep the highest version of each, and emit the
# result in sorted order so the classpath is a pure function of what is installed.

# mcgen_classpath <libraries-dir> [exclude-pattern ...]
#
# Prints a colon-separated classpath on stdout.
mcgen_classpath() {
  local libs="$1"
  shift || true

  local find_args=("$libs" -name '*.jar' ! -name '*-sources.jar' ! -name '*-javadoc.jar')
  local ex
  for ex in "$@"; do
    find_args+=(! -name "$ex")
  done

  # Key each jar by its artifact directory, which is everything above the version
  # directory. Pick the version-sorted maximum per artifact, then keep *every* jar
  # in that winning version directory -- a single version directory can legitimately
  # hold several jars, and Minecraft's does: the server ships -srg, -slim, -extra
  # and -unpacked in one directory. Keeping only one of them silently removes the
  # Minecraft classes from the classpath.
  find "${find_args[@]}" \
    | awk -F/ '{ v = $(NF-1); a = ""; for (i = 1; i < NF-1; i++) a = a "/" $i; print a "\t" v "\t" $0 }' \
    | sort -t$'\t' -k1,1 -k2,2V \
    | awk -F'\t' '
        { win[$1] = $2; row[NR] = $0; n = NR }
        END {
          for (i = 1; i <= n; i++) {
            split(row[i], f, "\t")
            if (f[2] == win[f[1]]) print f[3]
          }
        }' \
    | sort \
    | paste -sd:
}
