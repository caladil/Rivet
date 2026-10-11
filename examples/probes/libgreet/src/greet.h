#ifndef RIVET_GREET_H
#define RIVET_GREET_H

/* Set by the build. Two builds that differ only in this produce two libraries
 * with different digests and therefore different install directories, which is
 * the property worth demonstrating. */
#ifndef GREET_VERSION
#define GREET_VERSION "0.0.0"
#endif

const char *greet_who(void);
int greet_answer(void);

#endif
