# A machine with ssh and no Hof, for `mise run remote-test`. The base is
# a build argument, so the same test can run on other systems.
ARG BASE=debian:bookworm-slim
FROM ${BASE}
RUN if command -v apt-get >/dev/null; then \
      apt-get update && apt-get install -y --no-install-recommends openssh-server && rm -rf /var/lib/apt/lists/*; \
    else \
      apk add --no-cache openssh-server bash; \
    fi \
 && mkdir -p /run/sshd \
 && ssh-keygen -A \
 && (useradd -m -s /bin/bash me 2>/dev/null || adduser -D -s /bin/bash me) \
 && (passwd -d me >/dev/null 2>&1 || true) \
 && mkdir -p /home/me/.ssh && chmod 700 /home/me/.ssh
COPY key.pub /home/me/.ssh/authorized_keys
RUN chown -R me /home/me/.ssh && chmod 600 /home/me/.ssh/authorized_keys
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
