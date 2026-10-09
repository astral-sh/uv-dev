#include <Python.h>

int main(void) {
    Py_Initialize();
    int result = PyRun_SimpleString(
        "import sys, ssl, sqlite3\n"
        "print('Embedded Python:', sys.version)\n"
        "assert sqlite3.connect(':memory:').execute('select 42').fetchone() == (42,)\n"
    );
    return Py_FinalizeEx() < 0 ? 120 : result != 0;
}
