/* The first Shared Rivet. Deliberately trivial: what is being proved is the
 * mechanism — that a signed, content-addressed shared object installs onto a
 * board and that an application declaring it resolves to exactly those bytes —
 * and a library with any real behaviour in it would make a failure ambiguous.
 *
 * It carries a version string so that a board can be asked WHICH copy answered,
 * which is the question content addressing exists to make answerable. */

#include "greet.h"

const char *greet_who(void) {
    return "libgreet " GREET_VERSION " — resolved through a Shared Rivet";
}

int greet_answer(void) { return 42; }
