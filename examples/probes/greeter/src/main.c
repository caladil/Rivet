/* The first application to resolve a Shared Rivet.
 *
 * It is C rather than Rust deliberately. `ADR-015` §3.1 has said since
 * 2026-08-22 that "a static C or Go binary is a valid package and will work",
 * and that sentence has never had an artifact behind it. This is that artifact,
 * one step further out: a DYNAMIC C binary, resolving a library that arrived as
 * its own signed .rvt.
 *
 * What it prints is what a person needs to see to believe the mechanism:
 * the answer came from the library, and the library came from a path whose name
 * is a digest of its own bytes. */

#include <stdio.h>
#include <unistd.h>

#include "greet.h"

int main(void) {
    unsigned n = 0;
    for (;;) {
        /* Both calls cross into libgreet.so.1, which the loader found through
         * the LD_LIBRARY_PATH rivet-sysd composed from this application's
         * declared closure and from nothing else. */
        printf("greeter: tick %u — %s (answer %d)\n", n, greet_who(), greet_answer());
        fflush(stdout);
        n++;
        sleep(30);
    }
}
