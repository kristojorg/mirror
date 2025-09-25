# TODO
- In create_resource_link, we are currently skipping the resolution of the resource url if it is a local path. We need to stop doing that, and also stop saying absolute_url in the resource. Maybe we need to return a LocalResource and a RemoteResource.
  - We should also resolve the local paths at least to be from the output directory there, since we have the path to the local file it was found in. Then that is all we need to look up the resource.
- Proxy support
- Allow using random user agent from a file/list
- Make sure filters work.
- Make sure wait and max connections works.

## Notes

- Maybe rip our Webp
