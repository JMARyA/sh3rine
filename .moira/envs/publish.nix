{
  pkgs ? import <nixpkgs> { },
}:
pkgs.mkShell {
  buildInputs = [ pkgs.skopeo ];

shellHook = ''
  cat > .containers-policy.json <<EOF
  {
    "default": [
      { "type": "insecureAcceptAnything" }
    ]
  }
  EOF
'';

}
