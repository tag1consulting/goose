# GooseBot: AI-Powered Code Reviews for Goose

GooseBot is an AI-powered code review assistant for the Goose load testing framework. It uses Anthropic's Claude Sonnet model to provide automated feedback on pull requests.

## Features

- Reviews PR clarity and documentation
- Integrates with GitHub Actions workflow
- Reads project context from memory-bank files
- Filters files based on configurable patterns
- Posts reviews as PR comments

## Local Testing

### Setup

```bash
# 1. Create and activate a virtual environment
python3 -m venv .venv
source .venv/bin/activate  # On macOS/Linux

# 2. Install the required dependencies
pip install -r .github/scripts/requirements.txt

# 3. Create a .env file in the project root
echo "GITHUB_TOKEN=your_github_token_here" > .env
echo "ANTHROPIC_API_KEY=your_anthropic_key_here" >> .env
```

### Usage

```bash
# Run with PR number
.github/scripts/test_goosebot.py 616

# Full GitHub PR URL
.github/scripts/test_goosebot.py https://github.com/tag1consulting/goose/pull/616

# Custom prompt file
.github/scripts/test_goosebot.py 616 --prompt-file my_prompt.md 

# Show full prompt (debug mode)
.github/scripts/test_goosebot.py 616 --debug
```

## Setup Instructions

### 1. GitHub Secrets Configuration

To use GooseBot, you need to set up the following GitHub Secret:

1. Go to your GitHub repository page
2. Click on "Settings" (top navigation bar)
3. In the left sidebar, click on "Secrets and variables" then "Actions"
4. Click the "New repository secret" button
5. Add the following secrets:
   - Name: `ANTHROPIC_API_KEY`
   - Value: Your Anthropic API key
   
   - Name: `ANTHROPIC_API_URL`
   - Value: Your internal Anthropic API URL (for self-hosted Claude)

The `GITHUB_TOKEN` is automatically provided by GitHub Actions and does not need to be manually configured.

### 2. Workflow Configuration

The GooseBot workflow is defined in `.github/workflows/goosebot_review.yml`. You can customize it by modifying:

- File filtering patterns (`PR_REVIEW_WHITELIST` and `PR_REVIEW_BLACKLIST`)
- Token budget limits
- Review scope and other parameters

## Usage

GooseBot runs automatically on new PRs and PR updates. You can also trigger it manually:

1. Go to the "Actions" tab in your repository
2. Select "GooseBot PR Review" from the workflows list
3. Click "Run workflow"
4. Enter the PR number and optionally the review scope
5. Click "Run workflow" to start the review

## Customizing Reviews

To customize the review process:

- Edit prompt templates in `.github/prompts/v1/`
- Add new review scopes by creating additional prompt files
- Modify the file filtering patterns in the workflow file

## Prompt Development

GooseBot uses versioned prompt templates stored in the `.github/prompts/` directory. To create a new review scope:

1. Create a new file in `.github/prompts/v1/` named `<scope>_review.md`
2. Format it following the existing clarity review template
3. Update the workflow to use your new scope

## Troubleshooting

If GooseBot encounters issues:

- Check the GitHub Actions logs for detailed error messages
- Verify that your ANTHROPIC_API_KEY is correctly set
- Make sure the PR contains files that match the whitelist patterns
- Check that anthropic and PyGithub dependencies are correctly installed
