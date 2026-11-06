FROM python@sha256:b18b261e1b4974b4f321451ad5251ded85abb22c2b8b262da527dda1d031ab30
ENV UV_EXCLUDE_NEWER=2026-10-01T00:00:00Z
RUN --mount=type=bind,source=debs,target=/debs,readonly \
    DEBIAN_FRONTEND=noninteractive dpkg --install /debs/*.deb \
    && apt-get clean \
    && pkg-config --modversion libavformat libavcodec libavdevice libavutil libavfilter libswscale libswresample
