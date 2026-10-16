#include <signal.h>
#include <string.h>
#include <unistd.h>

static void owned_signal_site(void) {
    raise(SIGSEGV);
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "fault") == 0) {
        owned_signal_site();
    }
    if (argc > 1 && strcmp(argv[1], "wait") == 0) {
        for (;;) {
            pause();
        }
    }
    return 0;
}
