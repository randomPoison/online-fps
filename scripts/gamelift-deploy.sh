set -x

# Make sure the AWS CLI is installed.
pip install --user awscli

# Upload the contents of the bin directory as a build to GameLift.
aws gamelift upload-build \
  --operating-system AMAZON_LINUX \
  --build-root ./bin \
  --name "Test Build" \
  --build-version "0.0.$TRAVIS_BUILD_NUMBER" \
  --region us-east-1
