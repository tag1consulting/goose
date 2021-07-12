# The Internet

This example runs Goose against a local (container) version of ['the-internet'](https://github.com/saucelabs/the-internet) - we don't think it polite to hammer someones [Heroku app](http://the-internet.herokuapp.com/) with a tool as aggressive as a Goose can be.

The Dockerfile.goose will build the Goose container.  Please feel free to extend the endpoints tested by contributing a rust file and adding a corresponding `RUN` statement in the Dockerfile.

## Podman

NOTE: While Podman runs as an ordinary user by default, the following assumes you run your containers under an appropriately 'jailed' user account.

Build containers for Goose The-Internet:

```bash
podman build --rm --tag the-internet --file Dockerfile/the-internet .
podman build --rm --tag gti --file Dockerfile/gti .
```

Launch containers for Goose The-Internet:

- Podman users can try `podman-compose up` or `podman play kube ./podman-pod.yaml`
- Kubernetes users can use:

    ```bash
    k3s kubectl create -f ./k8s-pod.yml
    minikube kubectl -- create -f ./k8s-pod.yml
    ```

## Docker

NOTE: Since Docker runs as rootfully by default we discourage using any container in this mode.  The following assumes you run your conatiners under an appropriately 'jailed' user account.

Build containers for Goose The-Internet:

```bash
docker build --rm --tag the-internet --file Dockerfile/the-internet .
docker build --rm --tag gti --file Dockerfile/gti .
```

Launch containers for Goose The-Internet:

- Docker users can used `docker-compose up`.
