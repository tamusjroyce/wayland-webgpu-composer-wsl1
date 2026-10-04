/* WSL1 memfd_create() shim.
 *
 * WSL1's kernel emulation does not implement the memfd_create syscall, so
 * libwayland/Qt/KWin/foot abort with "Function not implemented" when they try
 * to create an anonymous shared-memory fd (keymaps, shm buffers, ...).
 *
 * This LD_PRELOAD library replaces the libc memfd_create wrapper with a
 * fallback that creates an unlinked temp file on a tmpfs-like directory and
 * returns its fd. mmap(MAP_SHARED)/ftruncate work; file sealing is not
 * supported (F_ADD_SEALS fails) but callers generally ignore that.
 *
 * Build:  gcc -shared -fPIC -O2 -o wwc-memfd-shim.so memfd_shim.c
 * Use:    LD_PRELOAD=/usr/local/lib/wwc-memfd-shim.so <program>
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#ifndef MFD_CLOEXEC
#define MFD_CLOEXEC 0x0001U
#endif

int memfd_create(const char *name, unsigned int flags)
{
	(void)name;
	const char *dirs[3];
	dirs[0] = getenv("XDG_RUNTIME_DIR");
	dirs[1] = "/dev/shm";
	dirs[2] = "/tmp";

	for (int i = 0; i < 3; i++) {
		if (!dirs[i] || !dirs[i][0])
			continue;
		char path[512];
		snprintf(path, sizeof(path), "%s/wwc-memfd-XXXXXX", dirs[i]);
		int fd = mkstemp(path);
		if (fd >= 0) {
			unlink(path); /* anonymous: vanishes on close */
			if (flags & MFD_CLOEXEC) {
				int fl = fcntl(fd, F_GETFD);
				if (fl != -1)
					fcntl(fd, F_SETFD, fl | FD_CLOEXEC);
			}
			return fd;
		}
	}
	return -1;
}
