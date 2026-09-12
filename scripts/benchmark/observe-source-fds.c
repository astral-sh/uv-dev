#include <errno.h>
#include <limits.h>
#include <libproc.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/proc_info.h>

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: observe-source-fds PID SOURCE_CACHE_PREFIX\n");
        return 2;
    }
    char *end = NULL;
    errno = 0;
    long parsed = strtol(argv[1], &end, 10);
    if (errno != 0 || end == argv[1] || *end != '\0' || parsed <= 0 || parsed > INT_MAX) {
        return 2;
    }
    int pid = (int)parsed;
    int required = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, NULL, 0);
    if (required <= 0) {
        return 1;
    }
    if (required > 16 * 1024 * 1024 - (int)(256 * sizeof(struct proc_fdinfo))) {
        return 2;
    }

    int capacity = required + (int)(256 * sizeof(struct proc_fdinfo));
    struct proc_fdinfo *files = NULL;
    int bytes = 0;
    for (;;) {
        if (capacity <= 0 || capacity > 16 * 1024 * 1024) {
            free(files);
            return 2;
        }
        struct proc_fdinfo *resized = realloc(files, (size_t)capacity);
        if (resized == NULL) {
            free(files);
            return 2;
        }
        files = resized;
        bytes = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, files, capacity);
        if (bytes <= 0) {
            free(files);
            return 1;
        }
        if (bytes < capacity) {
            break;
        }
        capacity *= 2;
    }

    int count = bytes / (int)sizeof(struct proc_fdinfo);
    int source_files = 0;
    int source_locks = 0;
    size_t prefix_length = strlen(argv[2]);
    const char suffix[] = "/.lock";
    for (int index = 0; index < count; index++) {
        if (files[index].proc_fdtype != PROX_FDTYPE_VNODE) {
            continue;
        }
        struct vnode_fdinfowithpath info = {0};
        if (proc_pidfdinfo(pid, files[index].proc_fd, PROC_PIDFDVNODEPATHINFO, &info,
                           (int)sizeof(info)) != (int)sizeof(info)) {
            continue;
        }
        size_t length = strnlen(info.pvip.vip_path, sizeof(info.pvip.vip_path));
        if (length < prefix_length || strncmp(info.pvip.vip_path, argv[2], prefix_length) != 0) {
            continue;
        }
        source_files++;
        if (length >= sizeof(suffix) - 1 &&
            memcmp(info.pvip.vip_path + length - (sizeof(suffix) - 1), suffix,
                   sizeof(suffix) - 1) == 0) {
            source_locks++;
        }
    }
    free(files);
    printf("{\"descriptors\":%d,\"source_cache_descriptors\":%d,"
           "\"source_lock_descriptors\":%d}\n",
           count, source_files, source_locks);
    return 0;
}
