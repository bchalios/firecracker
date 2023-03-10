import subprocess

class GitWorktree(object):
    def __init__(self, git_ref):
        self.__git_ref = git_ref

    def __enter__(self):
        result = subprocess.run(["git", "rev-parse", self.__git_ref], stdout=subprocess.PIPE, text=True, check=True)
        hash = result.stdout.strip()
        subprocess.run(["git", "worktree", "add", hash, hash], stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
        self._hash = hash
        return self._hash

    def __exit__(self, *args):
        subprocess.run(["git", "worktree", "remove", self._hash], check=True)

