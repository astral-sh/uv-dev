#include <Python.h>
#include "generated.h"
#include "target.h"

static PyObject *answer(PyObject *self, PyObject *args) {
    return PyLong_FromLong(GENERATED_VALUE * TARGET_VALUE);
}

static PyMethodDef methods[] = {
    {"answer", answer, METH_NOARGS, "Return the value computed from build and target inputs."},
    {NULL, NULL, 0, NULL}
};

static struct PyModuleDef module = {
    PyModuleDef_HEAD_INIT, "_answer", NULL, -1, methods
};

PyMODINIT_FUNC PyInit__answer(void) {
    return PyModule_Create(&module);
}
