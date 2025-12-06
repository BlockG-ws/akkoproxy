# Contributing to Akkoma Media Proxy

Thank you for your interest in contributing to Akkoma Media Proxy! This document provides guidelines and instructions for contributing.

## Getting Started

### Prerequisites

- Rust 1.70 or later
- Git
- Basic knowledge of Rust and HTTP protocols

### Development Setup

1. Clone the repository:
```bash
git clone https://github.com/BlockG-ws/fantastic-computing-machine.git
cd fantastic-computing-machine
```

2. Build the project:
```bash
cargo build
```

3. Run tests:
```bash
cargo test
```

4. Run the application:
```bash
UPSTREAM_URL=https://example.com cargo run
```

## Development Workflow

### Making Changes

1. Create a new branch for your feature or bug fix:
```bash
git checkout -b feature/your-feature-name
```

2. Make your changes and ensure they follow the coding standards

3. Add tests for new functionality

4. Run the test suite:
```bash
cargo test
```

5. Run clippy for linting:
```bash
cargo clippy -- -D warnings
```

6. Format your code:
```bash
cargo fmt
```

### Testing

- Write unit tests for new functionality
- Ensure all tests pass before submitting
- Add integration tests for major features
- Test with real Akkoma/Pleroma instances when possible

### Code Style

- Follow Rust standard style guidelines
- Use `cargo fmt` to format code
- Use meaningful variable and function names
- Add comments for complex logic
- Keep functions focused and small

## Pull Request Process

1. Update documentation if needed
2. Add tests for new features
3. Ensure all tests pass
4. Update CHANGELOG.md if applicable
5. Submit a pull request with a clear description of changes

### Pull Request Guidelines

- Write clear, descriptive commit messages
- Reference related issues in PR description
- Keep PRs focused on a single feature or fix
- Respond to review feedback promptly

## Reporting Bugs

### Before Reporting

- Check if the bug has already been reported
- Try to reproduce the bug with the latest version
- Collect relevant information (logs, configuration, etc.)

### Bug Report Template

```
**Description**
A clear description of the bug

**Steps to Reproduce**
1. Step 1
2. Step 2
3. ...

**Expected Behavior**
What you expected to happen

**Actual Behavior**
What actually happened

**Environment**
- OS: [e.g., Ubuntu 22.04]
- Rust version: [e.g., 1.70]
- Application version: [e.g., 0.1.0]

**Additional Context**
Any other relevant information
```

## Feature Requests

We welcome feature requests! Please:

- Check if the feature has already been requested
- Clearly describe the feature and its use case
- Explain how it would benefit the project
- Be open to discussion and feedback

## Code Review Process

- At least one maintainer will review your PR
- Address review feedback promptly
- Be patient - reviews may take time
- Be respectful and professional

## Security

If you discover a security vulnerability, please email the maintainers privately instead of opening a public issue.

## License

By contributing, you agree that your contributions will be licensed under the same license as the project (MIT OR Apache-2.0).

## Questions?

Feel free to open an issue for questions or reach out to the maintainers.

## Thank You!

Your contributions make this project better for everyone!
